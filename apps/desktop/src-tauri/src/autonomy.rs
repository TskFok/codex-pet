//! DeepSeek API 驱动的桌宠桌面自由活动模块。
//!
//! 本模块严格限定仅用于桌宠的自主活动决策（漫步、巡逻、跳跃、招手、休息、张望及短句自言自语），
//! 不接触也不向外部发送任何会话内容、代码、项目名或系统通知详情。

use crate::{
    NotificationRuntime, notification_paths,
    placement::{DesktopContext, WindowPlacement},
    platform,
    settings::{DEEPSEEK_DEFAULT_MODEL, DEFAULT_PET, Preferences, valid_deepseek_model},
};
use pet_core::paths::wall_clock_ms;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::watch;

pub const DEEPSEEK_CHAT_COMPLETIONS_URL: &str = "https://api.deepseek.com/chat/completions";

const MIN_PAUSE_MS: u64 = 10_000;
const MAX_PAUSE_MS: u64 = 35_000;
const DEFAULT_PAUSE_MS: u64 = 16_000;
const USER_INTERACTION_COOLDOWN_MS: u64 = 7_000;
const MOVE_STEP_MS: u64 = 33;
const WALK_SPEED_PX_PER_SEC: f64 = 175.0;
const MIN_WALK_DURATION_MS: u64 = 900;
const MAX_WALK_DURATION_MS: u64 = 4_200;
const MAX_THOUGHT_CHARS: usize = 36;
const MAX_HISTORY_ITEMS: usize = 6;
const INITIAL_RETRY_BACKOFF: Duration = Duration::from_secs(15);
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekActivityStatus {
    pub configured: bool,
    pub enabled: bool,
    pub running: bool,
    pub model: String,
    pub last_action: Option<String>,
    pub last_thought: Option<String>,
    pub last_updated_at_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl Default for DeepSeekActivityStatus {
    fn default() -> Self {
        Self {
            configured: false,
            enabled: true,
            running: false,
            model: DEEPSEEK_DEFAULT_MODEL.into(),
            last_action: None,
            last_thought: None,
            last_updated_at_ms: None,
            last_error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekTestResult {
    pub ok: bool,
    pub model: String,
    pub latency_ms: u64,
    pub reply: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityAction {
    Walk,
    Patrol,
    Hop,
    Wave,
    Rest,
    LookAround,
}

impl ActivityAction {
    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Walk => "桌面漫步",
            Self::Patrol => "桌面巡逻",
            Self::Hop => "开心蹦跳",
            Self::Wave => "招手致意",
            Self::Rest => "安静小憩",
            Self::LookAround => "东张西望",
        }
    }

    pub fn api_name(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Patrol => "patrol",
            Self::Hop => "hop",
            Self::Wave => "wave",
            Self::Rest => "rest",
            Self::LookAround => "look_around",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PetActivityPlan {
    pub action: ActivityAction,
    pub target_x: f64,
    pub target_y: f64,
    pub duration_ms: u64,
    pub pause_ms: u64,
    pub effect: Option<String>,
    pub thought: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEffectPayload {
    pub x: f64,
    pub y: f64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounce: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u8>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetActivityEvent {
    pub id: String,
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<ActivityEffectPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_duration_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ActivityPromptInput {
    pub pet_name: String,
    pub work_area_width: u32,
    pub work_area_height: u32,
    pub current_norm_x: f64,
    pub current_norm_y: f64,
    pub active_minutes: u64,
    pub recent_actions: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DeepSeekError {
    pub message: String,
    pub fatal_auth: bool,
}

impl std::fmt::Display for DeepSeekError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPlan {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    target_x: Option<f64>,
    #[serde(default)]
    target_y: Option<f64>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    pause_ms: Option<u64>,
    #[serde(default)]
    effect: Option<String>,
    #[serde(default)]
    thought: Option<String>,
}

pub struct DeepSeekClient {
    http: reqwest::Client,
    endpoint: String,
}

impl DeepSeekClient {
    pub fn new() -> Result<Self, DeepSeekError> {
        Self::with_endpoint(DEEPSEEK_CHAT_COMPLETIONS_URL)
    }

    pub fn with_endpoint(endpoint: impl Into<String>) -> Result<Self, DeepSeekError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| DeepSeekError {
                message: format!("无法初始化网络客户端：{e}"),
                fatal_auth: false,
            })?;
        Ok(Self {
            http,
            endpoint: endpoint.into(),
        })
    }

    pub async fn plan_activity(
        &self,
        api_key: &str,
        model: &str,
        input: &ActivityPromptInput,
    ) -> Result<PetActivityPlan, DeepSeekError> {
        let system_prompt = build_system_prompt();
        let user_prompt = build_user_prompt(input);
        let content = self
            .complete(api_key, model, system_prompt, &user_prompt, true)
            .await?;
        parse_activity_plan(&content, input.current_norm_x, input.current_norm_y)
    }

    pub async fn test_connection(
        &self,
        api_key: &str,
        model: &str,
    ) -> Result<DeepSeekTestResult, DeepSeekError> {
        let trimmed_key = api_key.trim();
        if trimmed_key.is_empty() {
            return Err(DeepSeekError {
                message: "请先填写 DeepSeek API Key".into(),
                fatal_auth: true,
            });
        }
        let trimmed_model = model.trim();
        if !valid_deepseek_model(trimmed_model) {
            return Err(DeepSeekError {
                message: format!("不支持的 DeepSeek 模型：{trimmed_model}"),
                fatal_auth: true,
            });
        }
        let started = Instant::now();
        let content = self
            .complete(
                trimmed_key,
                trimmed_model,
                "你是一只可爱、简短回应的桌面宠物。只回复一句不超过16字的中文打招呼。",
                "测试连接，请向主人打个招呼。",
                false,
            )
            .await?;
        let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let reply = sanitize_thought(&content).unwrap_or_else(|| "连接成功，随时可以自由活动！".into());
        Ok(DeepSeekTestResult {
            ok: true,
            model: trimmed_model.into(),
            latency_ms,
            reply,
        })
    }

    async fn complete(
        &self,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        user_prompt: &str,
        expect_json: bool,
    ) -> Result<String, DeepSeekError> {
        let is_reasoner = model == "deepseek-reasoner";
        let timeout = if is_reasoner {
            Duration::from_secs(60)
        } else {
            Duration::from_secs(25)
        };
        let request = ChatCompletionRequest {
            model,
            messages: [
                ChatMessage {
                    role: "system",
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: user_prompt,
                },
            ],
            response_format: if expect_json && !is_reasoner {
                Some(ResponseFormat {
                    kind: "json_object",
                })
            } else {
                None
            },
            temperature: if is_reasoner { None } else { Some(1.0) },
            max_tokens: if is_reasoner {
                1024
            } else if expect_json {
                240
            } else {
                64
            },
            stream: false,
        };

        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(api_key)
            .timeout(timeout)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    DeepSeekError {
                        message: "请求 DeepSeek API 超时，请检查网络连接".into(),
                        fatal_auth: false,
                    }
                } else {
                    DeepSeekError {
                        message: "连接 DeepSeek API 失败，请检查网络或代理设置".into(),
                        fatal_auth: false,
                    }
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            return Err(match code {
                401 => DeepSeekError {
                    message: "DeepSeek API Key 无效或已过期（HTTP 401）".into(),
                    fatal_auth: true,
                },
                402 => DeepSeekError {
                    message: "DeepSeek 账户余额不足（HTTP 402）".into(),
                    fatal_auth: true,
                },
                429 => DeepSeekError {
                    message: "DeepSeek API 请求过于频繁（HTTP 429），稍后重试".into(),
                    fatal_auth: false,
                },
                400 | 404 | 422 => DeepSeekError {
                    message: format!("DeepSeek API 请求参数或模型不受支持（HTTP {code}）"),
                    fatal_auth: false,
                },
                _ => DeepSeekError {
                    message: format!("DeepSeek 服务暂时异常（HTTP {code}），稍后重试"),
                    fatal_auth: false,
                },
            });
        }

        let payload: ChatCompletionResponse =
            response.json().await.map_err(|_| DeepSeekError {
                message: "DeepSeek API 响应格式无效".into(),
                fatal_auth: false,
            })?;
        let text = payload
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .unwrap_or_default();
        if text.trim().is_empty() {
            return Err(DeepSeekError {
                message: "DeepSeek API 返回了空内容".into(),
                fatal_auth: false,
            });
        }
        Ok(text)
    }
}

pub fn build_system_prompt() -> &'static str {
    "你是桌面宠物的自主行为控制器，仅负责决定桌宠在电脑桌面上的自由活动（走动、巡逻、蹦跳、招手、小憩、张望）和简短自言自语。\n\
请严格只输出一个 JSON 对象，不要包含额外解释，字段如下：\n\
- action: 字符串，必须是 \"walk\" | \"patrol\" | \"hop\" | \"wave\" | \"rest\" | \"look_around\" 之一；优先在桌面上多自由走动（walk/patrol/hop）并穿插原地动作。\n\
- targetX: 0.05 到 0.95 之间的小数，表示目标横向位置比例（0 在桌面最左，1 在最右）。\n\
- targetY: 0.08 到 0.94 之间的小数，表示目标纵向位置比例（0 在桌面最上，1 在最下；桌宠喜欢待在桌面边缘或中下方，也可以偶尔逛去上方）。\n\
- durationMs: 动作持续毫秒数（1500 到 5000）。\n\
- pauseMs: 本次活动结束后休息毫秒数（12000 到 30000）。\n\
- effect: \"sparkle\" | \"heart\" | \"note\" | \"star\" | \"none\" 之一。\n\
- thought: 可选的简短中文自言自语（不超过24字，语气可爱自然；偶尔可填空字符串 \"\" 保持安静不打扰）。"
}

pub fn build_user_prompt(input: &ActivityPromptInput) -> String {
    let history = if input.recent_actions.is_empty() {
        "暂无（刚醒来，适合先在桌面上自由逛一逛）".to_owned()
    } else {
        input.recent_actions.join(" -> ")
    };
    format!(
        "宠物名称：{}\n桌面活动区域：{}x{}（逻辑像素）\n当前位置比例：x={:.2}, y={:.2}\n已自由活动时长：{} 分钟\n最近活动记录：{}\n请规划下一次桌宠自由活动，避免反复停留在相同位置或重复同一句话。",
        input.pet_name,
        input.work_area_width,
        input.work_area_height,
        input.current_norm_x.clamp(0.0, 1.0),
        input.current_norm_y.clamp(0.0, 1.0),
        input.active_minutes,
        history
    )
}

fn strip_think_blocks(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find("<think>") {
        output.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("</think>") {
            rest = &rest[start + end + "</think>".len()..];
        } else {
            return output;
        }
    }
    output.push_str(rest);
    output
}

fn extract_json_object(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let start = trimmed.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in trimmed[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&trimmed[start..=start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

fn sanitize_thought(raw: &str) -> Option<String> {
    let cleaned = strip_think_blocks(raw);
    let compact: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c| matches!(c, '"' | '\'' | '「' | '」' | '`'))
        .trim()
        .to_owned();
    if compact.is_empty() {
        return None;
    }
    let mut truncated: String = compact.chars().take(MAX_THOUGHT_CHARS).collect();
    if compact.chars().count() > MAX_THOUGHT_CHARS {
        truncated.push('…');
    }
    Some(truncated)
}

fn normalize_effect(raw: Option<String>) -> Option<String> {
    match raw.as_deref().map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("heart") => Some("heart".into()),
        Some("sparkle") => Some("sparkle".into()),
        Some("note") => Some("note".into()),
        Some("star") => Some("star".into()),
        _ => None,
    }
}

pub fn parse_activity_plan(
    raw_content: &str,
    fallback_x: f64,
    fallback_y: f64,
) -> Result<PetActivityPlan, DeepSeekError> {
    let stripped = strip_think_blocks(raw_content);
    let json_slice = extract_json_object(&stripped).ok_or_else(|| DeepSeekError {
        message: "DeepSeek 返回的活动指令格式无法解析".into(),
        fatal_auth: false,
    })?;
    let parsed: RawPlan = serde_json::from_str(json_slice).map_err(|_| DeepSeekError {
        message: "DeepSeek 返回的活动 JSON 无法解析".into(),
        fatal_auth: false,
    })?;
    let action = match parsed
        .action
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("walk" | "move" | "roam") => ActivityAction::Walk,
        Some("patrol" | "stroll") => ActivityAction::Patrol,
        Some("hop" | "jump") => ActivityAction::Hop,
        Some("wave" | "greet") => ActivityAction::Wave,
        Some("rest" | "wait" | "idle") => ActivityAction::Rest,
        Some("look_around" | "look" | "review" | "explore") => ActivityAction::LookAround,
        _ => ActivityAction::Walk,
    };
    let target_x = parsed
        .target_x
        .filter(|v| v.is_finite())
        .unwrap_or(fallback_x)
        .clamp(0.03, 0.97);
    let target_y = parsed
        .target_y
        .filter(|v| v.is_finite())
        .unwrap_or(fallback_y)
        .clamp(0.05, 0.96);
    let duration_ms = parsed.duration_ms.unwrap_or(2_800).clamp(1_200, 6_000);
    let pause_ms = parsed
        .pause_ms
        .unwrap_or(DEFAULT_PAUSE_MS)
        .clamp(MIN_PAUSE_MS, MAX_PAUSE_MS);
    let effect = normalize_effect(parsed.effect);
    let thought = parsed.thought.as_deref().and_then(sanitize_thought);
    Ok(PetActivityPlan {
        action,
        target_x,
        target_y,
        duration_ms,
        pause_ms,
        effect,
        thought,
    })
}

fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub struct AutonomyRuntime {
    status: RwLock<DeepSeekActivityStatus>,
    interrupt_gen: AtomicU64,
    last_interaction_ms: AtomicU64,
    manual_trigger: AtomicBool,
    wake: watch::Sender<u64>,
    exit: watch::Sender<bool>,
    worker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl Default for AutonomyRuntime {
    fn default() -> Self {
        let (wake, _) = watch::channel(0u64);
        let (exit, _) = watch::channel(false);
        Self {
            status: RwLock::new(DeepSeekActivityStatus::default()),
            interrupt_gen: AtomicU64::new(0),
            last_interaction_ms: AtomicU64::new(0),
            manual_trigger: AtomicBool::new(false),
            wake,
            exit,
            worker: Mutex::new(None),
        }
    }
}

impl AutonomyRuntime {
    pub fn status(&self) -> DeepSeekActivityStatus {
        self.status.read().unwrap().clone()
    }

    fn set_status(&self, app: &tauri::AppHandle, next: DeepSeekActivityStatus) {
        let changed = {
            let mut current = self.status.write().unwrap();
            if *current == next {
                false
            } else {
                *current = next.clone();
                true
            }
        };
        if changed {
            let _ = app.emit("pet:activity-status", next);
        }
    }

    pub fn notify_wake(&self) {
        let next = self.wake.borrow().wrapping_add(1);
        let _ = self.wake.send(next);
    }

    pub fn notify_user_interaction(&self) {
        self.interrupt_gen.fetch_add(1, Ordering::SeqCst);
        self.last_interaction_ms
            .store(wall_clock_ms(), Ordering::SeqCst);
    }

    pub fn request_manual_trigger(&self) {
        self.manual_trigger.store(true, Ordering::SeqCst);
        self.notify_wake();
    }

    pub fn stop(&self) {
        self.interrupt_gen.fetch_add(1, Ordering::SeqCst);
        self.exit.send_replace(true);
        self.notify_wake();
    }
}

pub(crate) fn wake(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AutonomyRuntime>() {
        state.notify_wake();
    }
}

pub(crate) fn on_settings_updated(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AutonomyRuntime>() {
        state.notify_wake();
    }
}

pub(crate) fn interrupt_user_activity(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AutonomyRuntime>() {
        state.notify_user_interaction();
    }
}

pub(crate) fn shutdown(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AutonomyRuntime>() {
        state.stop();
        let worker = state.worker.lock().unwrap().take();
        if let Some(worker) = worker {
            let _ = tauri::async_runtime::block_on(worker);
        }
    }
}

pub(crate) fn start(app: tauri::AppHandle) {
    if std::env::var_os("PET_VALIDATION_STATE").is_some()
        || std::env::var("PET_VALIDATION_PHASE").is_ok_and(|p| p == "idle" || p == "hidden")
    {
        return;
    }
    let target = app.clone();
    let worker = tauri::async_runtime::spawn(async move {
        run_loop(app).await;
    });
    *target.state::<AutonomyRuntime>().worker.lock().unwrap() = Some(worker);
}

fn load_preferences_and_pet_name() -> (Preferences, String) {
    let Ok(paths) = notification_paths() else {
        return (Preferences::default(), "月影".into());
    };
    let prefs = Preferences::load(&paths).unwrap_or_default();
    let name = if prefs.pet_id == DEFAULT_PET {
        "月影".into()
    } else {
        pet_core::pet_package::describe_pet(&paths.pets.join(&prefs.pet_id))
            .map(|manifest| manifest.name)
            .unwrap_or_else(|_| "桌宠".into())
    };
    (prefs, name)
}

fn is_pet_ready_for_autonomy(app: &tauri::AppHandle) -> bool {
    let playback = app.state::<platform::Playback>().snapshot();
    if !playback.visible || playback.suspended {
        return false;
    }
    if let Some(runtime) = app.try_state::<NotificationRuntime>()
        && runtime.has_urgent_attention()
    {
        return false;
    }
    true
}

async fn wait_interruptible(
    duration: Duration,
    wake_rx: &mut watch::Receiver<u64>,
    exit_rx: &mut watch::Receiver<bool>,
) -> bool {
    if *exit_rx.borrow() {
        return false;
    }
    tokio::select! {
        biased;
        _ = exit_rx.wait_for(|stopped| *stopped) => false,
        _ = wake_rx.changed() => true,
        _ = tokio::time::sleep(duration) => true,
    }
}

async fn wait_for_wake(
    wake_rx: &mut watch::Receiver<u64>,
    exit_rx: &mut watch::Receiver<bool>,
) -> bool {
    if *exit_rx.borrow() {
        return false;
    }
    tokio::select! {
        biased;
        _ = exit_rx.wait_for(|stopped| *stopped) => false,
        _ = wake_rx.changed() => true,
    }
}

fn emit_activity(app: &tauri::AppHandle, event: &PetActivityEvent) {
    let _ = app.emit_to("pet", "pet:activity", event);
}

fn effect_payload(kind: Option<&str>, bounce: f64) -> Option<ActivityEffectPayload> {
    kind.map(|kind| ActivityEffectPayload {
        x: 0.5,
        y: 0.62,
        kind: kind.to_owned(),
        bounce: Some(bounce),
        count: Some(5),
    })
}

async fn execute_plan(
    app: &tauri::AppHandle,
    state: &AutonomyRuntime,
    ctx: &DesktopContext,
    plan: &PetActivityPlan,
    activity_id: &str,
    start_gen: u64,
    exit_rx: &mut watch::Receiver<bool>,
) {
    let interrupted = |exit: &watch::Receiver<bool>| -> bool {
        *exit.borrow()
            || state.interrupt_gen.load(Ordering::SeqCst) != start_gen
            || !is_pet_ready_for_autonomy(app)
    };

    match plan.action {
        ActivityAction::Walk | ActivityAction::Patrol => {
            let start_x = ctx.placement.x;
            let start_y = ctx.placement.y;
            let mut end_x = (plan.target_x * ctx.max_x).clamp(0.0, ctx.max_x);
            let end_y = (plan.target_y * ctx.max_y).clamp(0.0, ctx.max_y);
            let dist = (end_x - start_x).hypot(end_y - start_y);
            if dist < 48.0 && ctx.max_x > 120.0 {
                let shift = (ctx.max_x * 0.25).clamp(72.0, 260.0);
                end_x = if start_x + shift <= ctx.max_x {
                    start_x + shift
                } else {
                    (start_x - shift).max(0.0)
                };
            }
            let dist = (end_x - start_x).hypot(end_y - start_y);
            let run_state = if end_x >= start_x {
                "running-right"
            } else {
                "running-left"
            };
            let walk_ms = ((dist / WALK_SPEED_PX_PER_SEC) * 1000.0)
                .clamp(MIN_WALK_DURATION_MS as f64, MAX_WALK_DURATION_MS as f64)
                as u64;

            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "start",
                    state: Some(run_state),
                    effect: None,
                    thought: None,
                    thought_duration_ms: None,
                },
            );

            let started = Instant::now();
            let mut last_pos = ctx.placement.clone();
            loop {
                if interrupted(exit_rx) {
                    emit_activity(
                        app,
                        &PetActivityEvent {
                            id: activity_id.to_owned(),
                            phase: "end",
                            state: None,
                            effect: None,
                            thought: None,
                            thought_duration_ms: None,
                        },
                    );
                    return;
                }
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let progress = (elapsed_ms as f64 / walk_ms as f64).min(1.0);
                let eased = ease_in_out_cubic(progress);
                let next_pos = WindowPlacement {
                    x: start_x + (end_x - start_x) * eased,
                    y: start_y + (end_y - start_y) * eased,
                    scale: ctx.placement.scale,
                    monitor_id: ctx.placement.monitor_id.clone(),
                };
                let done = progress >= 1.0;
                if let Some(moved) = platform::step_placement(app, next_pos, done).await {
                    last_pos = moved;
                }
                if done {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = exit_rx.wait_for(|stopped| *stopped) => break,
                    _ = tokio::time::sleep(Duration::from_millis(MOVE_STEP_MS)) => {}
                }
            }

            if interrupted(exit_rx) {
                let _ = platform::step_placement(app, last_pos, true).await;
                emit_activity(
                    app,
                    &PetActivityEvent {
                        id: activity_id.to_owned(),
                        phase: "end",
                        state: None,
                        effect: None,
                        thought: None,
                        thought_duration_ms: None,
                    },
                );
                return;
            }

            let arrival_state = if plan.action == ActivityAction::Patrol {
                Some("waving")
            } else {
                Some("idle")
            };
            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "update",
                    state: arrival_state,
                    effect: effect_payload(plan.effect.as_deref(), 0.85),
                    thought: plan.thought.clone(),
                    thought_duration_ms: Some(3_800),
                },
            );
            if plan.action == ActivityAction::Patrol {
                tokio::select! {
                    biased;
                    _ = exit_rx.wait_for(|stopped| *stopped) => {}
                    _ = tokio::time::sleep(Duration::from_millis(900)) => {}
                }
            }
            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "end",
                    state: None,
                    effect: None,
                    thought: None,
                    thought_duration_ms: None,
                },
            );
        }
        ActivityAction::Hop => {
            let start_x = ctx.placement.x;
            let start_y = ctx.placement.y;
            let raw_dx = (plan.target_x * ctx.max_x - start_x).clamp(-200.0, 200.0);
            let raw_dy = (plan.target_y * ctx.max_y - start_y).clamp(-120.0, 120.0);
            let end_x = (start_x + raw_dx).clamp(0.0, ctx.max_x);
            let end_y = (start_y + raw_dy).clamp(0.0, ctx.max_y);
            let hop_ms = 760u64;

            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "start",
                    state: Some("jumping"),
                    effect: effect_payload(plan.effect.as_deref().or(Some("star")), 1.05),
                    thought: plan.thought.clone(),
                    thought_duration_ms: Some(3_400),
                },
            );

            let started = Instant::now();
            loop {
                if interrupted(exit_rx) {
                    break;
                }
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let t = (elapsed_ms as f64 / hop_ms as f64).min(1.0);
                let arc = (std::f64::consts::PI * t).sin() * 38.0;
                let next_pos = WindowPlacement {
                    x: start_x + (end_x - start_x) * t,
                    y: (start_y + (end_y - start_y) * t - arc).clamp(0.0, ctx.max_y),
                    scale: ctx.placement.scale,
                    monitor_id: ctx.placement.monitor_id.clone(),
                };
                let done = t >= 1.0;
                let _ = platform::step_placement(app, next_pos, done).await;
                if done {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = exit_rx.wait_for(|stopped| *stopped) => break,
                    _ = tokio::time::sleep(Duration::from_millis(MOVE_STEP_MS)) => {}
                }
            }
            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "end",
                    state: None,
                    effect: None,
                    thought: None,
                    thought_duration_ms: None,
                },
            );
        }
        ActivityAction::Wave | ActivityAction::Rest | ActivityAction::LookAround => {
            let (anim_state, default_effect, wait_ms) = match plan.action {
                ActivityAction::Wave => ("waving", Some("heart"), 1_000u64),
                ActivityAction::Rest => ("waiting", None, plan.duration_ms.clamp(2_000, 5_000)),
                ActivityAction::LookAround => {
                    ("review", Some("sparkle"), plan.duration_ms.clamp(2_000, 4_500))
                }
                _ => unreachable!(),
            };
            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "start",
                    state: Some(anim_state),
                    effect: effect_payload(plan.effect.as_deref().or(default_effect), 0.9),
                    thought: plan.thought.clone(),
                    thought_duration_ms: Some(3_600),
                },
            );
            let started = Instant::now();
            while started.elapsed().as_millis() < u128::from(wait_ms) {
                if interrupted(exit_rx) {
                    break;
                }
                tokio::select! {
                    biased;
                    _ = exit_rx.wait_for(|stopped| *stopped) => break,
                    _ = tokio::time::sleep(Duration::from_millis(120)) => {}
                }
            }
            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "end",
                    state: None,
                    effect: None,
                    thought: None,
                    thought_duration_ms: None,
                },
            );
        }
    }
}

async fn run_loop(app: tauri::AppHandle) {
    let state = app.state::<AutonomyRuntime>();
    let mut wake_rx = state.wake.subscribe();
    let mut exit_rx = state.exit.subscribe();
    let client = match DeepSeekClient::new() {
        Ok(client) => client,
        Err(error) => {
            state.set_status(
                &app,
                DeepSeekActivityStatus {
                    last_error: Some(error.message),
                    ..Default::default()
                },
            );
            return;
        }
    };

    let started_at = Instant::now();
    let mut recent_actions: VecDeque<String> = VecDeque::with_capacity(MAX_HISTORY_ITEMS);
    let mut backoff = INITIAL_RETRY_BACKOFF;
    let mut blocked_auth_key_model: Option<(String, String)> = None;
    let mut seq = 0u64;

    // 启动后先等待一小段缓冲，让桌宠窗口完成初始渲染。
    if !wait_interruptible(Duration::from_secs(3), &mut wake_rx, &mut exit_rx).await {
        return;
    }

    loop {
        if *exit_rx.borrow() {
            break;
        }
        let manual = state.manual_trigger.swap(false, Ordering::SeqCst);
        let (prefs, pet_name) = load_preferences_and_pet_name();
        let api_key = prefs.deepseek_api_key.trim().to_owned();
        let model = prefs.deepseek_model.clone();
        let configured = !api_key.is_empty();
        let enabled = prefs.deepseek_enabled;

        if !configured || !enabled {
            blocked_auth_key_model = None;
            backoff = INITIAL_RETRY_BACKOFF;
            let mut next = state.status();
            next.configured = configured;
            next.enabled = enabled;
            next.running = false;
            next.model = model;
            if !configured {
                next.last_error = None;
            }
            state.set_status(&app, next);
            if !wait_for_wake(&mut wake_rx, &mut exit_rx).await {
                break;
            }
            continue;
        }

        let current_pair = (api_key.clone(), model.clone());
        if !manual && blocked_auth_key_model.as_ref() == Some(&current_pair) {
            if !wait_for_wake(&mut wake_rx, &mut exit_rx).await {
                break;
            }
            continue;
        }
        if blocked_auth_key_model.as_ref() != Some(&current_pair) {
            blocked_auth_key_model = None;
            backoff = INITIAL_RETRY_BACKOFF;
        }

        if !is_pet_ready_for_autonomy(&app) {
            let mut next = state.status();
            next.configured = true;
            next.enabled = true;
            next.running = false;
            next.model = model;
            state.set_status(&app, next);
            if !wait_interruptible(Duration::from_secs(3), &mut wake_rx, &mut exit_rx).await {
                break;
            }
            continue;
        }

        if !manual {
            let last_interaction = state.last_interaction_ms.load(Ordering::SeqCst);
            let since = wall_clock_ms().saturating_sub(last_interaction);
            if last_interaction > 0 && since < USER_INTERACTION_COOLDOWN_MS {
                let remaining = Duration::from_millis(USER_INTERACTION_COOLDOWN_MS - since);
                if !wait_interruptible(remaining, &mut wake_rx, &mut exit_rx).await {
                    break;
                }
                continue;
            }
        }

        let Some(ctx) = platform::desktop_context(&app).await else {
            if !wait_interruptible(Duration::from_secs(3), &mut wake_rx, &mut exit_rx).await {
                break;
            }
            continue;
        };

        let mut running_status = state.status();
        running_status.configured = true;
        running_status.enabled = true;
        running_status.running = true;
        running_status.model = model.clone();
        state.set_status(&app, running_status);

        let norm_x = if ctx.max_x > 0.0 {
            ctx.placement.x / ctx.max_x
        } else {
            0.5
        };
        let norm_y = if ctx.max_y > 0.0 {
            ctx.placement.y / ctx.max_y
        } else {
            0.5
        };
        let prompt_input = ActivityPromptInput {
            pet_name,
            work_area_width: ctx.work_area_width.round().max(1.0) as u32,
            work_area_height: ctx.work_area_height.round().max(1.0) as u32,
            current_norm_x: norm_x,
            current_norm_y: norm_y,
            active_minutes: started_at.elapsed().as_secs() / 60,
            recent_actions: recent_actions.iter().cloned().collect(),
        };

        let start_gen = state.interrupt_gen.load(Ordering::SeqCst);
        let plan_result = tokio::select! {
            biased;
            _ = exit_rx.wait_for(|stopped| *stopped) => break,
            result = client.plan_activity(&api_key, &model, &prompt_input) => result,
        };

        match plan_result {
            Ok(plan) => {
                backoff = INITIAL_RETRY_BACKOFF;
                blocked_auth_key_model = None;
                let (latest_prefs, _) = load_preferences_and_pet_name();
                if !latest_prefs.deepseek_enabled
                    || latest_prefs.deepseek_api_key.trim().is_empty()
                {
                    continue;
                }
                if state.interrupt_gen.load(Ordering::SeqCst) == start_gen
                    && is_pet_ready_for_autonomy(&app)
                {
                    seq = seq.wrapping_add(1);
                    let activity_id = format!("deepseek-activity:{seq}");
                    execute_plan(
                        &app,
                        &state,
                        &ctx,
                        &plan,
                        &activity_id,
                        start_gen,
                        &mut exit_rx,
                    )
                    .await;
                    let summary = format!(
                        "{}({:.2},{:.2})",
                        plan.action.api_name(),
                        plan.target_x,
                        plan.target_y
                    );
                    if recent_actions.len() >= MAX_HISTORY_ITEMS {
                        recent_actions.pop_front();
                    }
                    recent_actions.push_back(summary);
                    state.set_status(
                        &app,
                        DeepSeekActivityStatus {
                            configured: true,
                            enabled: true,
                            running: true,
                            model: model.clone(),
                            last_action: Some(plan.action.label_zh().into()),
                            last_thought: plan.thought.clone(),
                            last_updated_at_ms: Some(wall_clock_ms()),
                            last_error: None,
                        },
                    );
                }
                if !wait_interruptible(
                    Duration::from_millis(plan.pause_ms),
                    &mut wake_rx,
                    &mut exit_rx,
                )
                .await
                {
                    break;
                }
            }
            Err(error) => {
                eprintln!("桌宠 DeepSeek 自由活动调用失败：{}", error.message);
                let mut failed = state.status();
                failed.configured = true;
                failed.enabled = true;
                failed.running = !error.fatal_auth;
                failed.model = model.clone();
                failed.last_updated_at_ms = Some(wall_clock_ms());
                failed.last_error = Some(error.message);
                state.set_status(&app, failed);

                if error.fatal_auth {
                    blocked_auth_key_model = Some(current_pair);
                    if !wait_for_wake(&mut wake_rx, &mut exit_rx).await {
                        break;
                    }
                } else {
                    let wait = backoff;
                    backoff = (backoff * 2).min(MAX_RETRY_BACKOFF);
                    if !wait_interruptible(wait, &mut wake_rx, &mut exit_rx).await {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::State, http::HeaderMap, http::StatusCode, routing::post};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    #[test]
    fn parses_standard_and_reasoner_outputs_with_strict_clamping() {
        let standard = r#"{"action":"walk","targetX":1.4,"targetY":-0.2,"durationMs":9000,"pauseMs":2000,"effect":"HEART","thought":"去右边晒晒太阳呀~"}"#;
        let plan = parse_activity_plan(standard, 0.5, 0.5).unwrap();
        assert_eq!(plan.action, ActivityAction::Walk);
        assert!((plan.target_x - 0.97).abs() < 1e-9);
        assert!((plan.target_y - 0.05).abs() < 1e-9);
        assert_eq!(plan.duration_ms, 6_000);
        assert_eq!(plan.pause_ms, MIN_PAUSE_MS);
        assert_eq!(plan.effect.as_deref(), Some("heart"));
        assert_eq!(plan.thought.as_deref(), Some("去右边晒晒太阳呀~"));

        let reasoned = "<think>我刚才在左边，现在跳到右边巡逻吧。</think>\n```json\n{\"action\":\"patrol\",\"targetX\":0.78,\"targetY\":0.84,\"pauseMs\":18000,\"effect\":\"sparkle\",\"thought\":\"巡逻桌面角落中\"}\n```";
        let plan2 = parse_activity_plan(reasoned, 0.2, 0.8).unwrap();
        assert_eq!(plan2.action, ActivityAction::Patrol);
        assert!((plan2.target_x - 0.78).abs() < 1e-9);
        assert!((plan2.target_y - 0.84).abs() < 1e-9);
        assert_eq!(plan2.pause_ms, 18_000);
        assert_eq!(plan2.effect.as_deref(), Some("sparkle"));
        assert_eq!(plan2.thought.as_deref(), Some("巡逻桌面角落中"));
    }

    #[test]
    fn prompt_only_contains_pet_desktop_activity_context() {
        let prompt = build_user_prompt(&ActivityPromptInput {
            pet_name: "月影".into(),
            work_area_width: 1440,
            work_area_height: 900,
            current_norm_x: 0.25,
            current_norm_y: 0.75,
            active_minutes: 12,
            recent_actions: vec!["walk(0.25,0.75)".into()],
        });
        assert!(prompt.contains("月影"));
        assert!(prompt.contains("1440x900"));
        assert!(prompt.contains("x=0.25, y=0.75"));
        assert!(prompt.contains("walk(0.25,0.75)"));
    }

    #[derive(Default)]
    struct MockServerState {
        captured: Mutex<Vec<(String, serde_json::Value)>>,
        next_status: Mutex<u16>,
        next_body: Mutex<String>,
    }

    async fn mock_chat_completions(
        State(state): State<Arc<MockServerState>>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        state.captured.lock().unwrap().push((auth, body));
        let status = *state.next_status.lock().unwrap();
        let content = state.next_body.lock().unwrap().clone();
        if status != 200 {
            return (
                StatusCode::from_u16(status).unwrap(),
                Json(serde_json::json!({ "error": { "message": "mock error" } })),
            );
        }
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "choices": [{
                    "message": { "role": "assistant", "content": content }
                }]
            })),
        )
    }

    #[tokio::test]
    async fn client_calls_deepseek_endpoint_for_chat_and_reasoner_and_handles_auth_errors() {
        let mock = Arc::new(MockServerState {
            captured: Mutex::new(Vec::new()),
            next_status: Mutex::new(200),
            next_body: Mutex::new(
                r#"{"action":"hop","targetX":0.62,"targetY":0.7,"durationMs":2000,"pauseMs":15000,"effect":"star","thought":"跳一跳活动筋骨！"}"#.into(),
            ),
        });
        let app = Router::new()
            .route("/chat/completions", post(mock_chat_completions))
            .with_state(mock.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client =
            DeepSeekClient::with_endpoint(format!("http://{addr}/chat/completions")).unwrap();
        let input = ActivityPromptInput {
            pet_name: "月影".into(),
            work_area_width: 1440,
            work_area_height: 900,
            current_norm_x: 0.4,
            current_norm_y: 0.7,
            active_minutes: 3,
            recent_actions: vec![],
        };

        // 1. deepseek-chat 请求带 response_format 和 temperature
        let plan = client
            .plan_activity("sk-test-secret-1234", "deepseek-chat", &input)
            .await
            .unwrap();
        assert_eq!(plan.action, ActivityAction::Hop);
        assert_eq!(plan.thought.as_deref(), Some("跳一跳活动筋骨！"));

        // 2. deepseek-reasoner 请求不发送 temperature 和 json response_format，可解析 <think> 输出
        *mock.next_body.lock().unwrap() =
            "<think>先想想去哪儿</think>{\"action\":\"wave\",\"targetX\":0.4,\"targetY\":0.7,\"pauseMs\":20000,\"effect\":\"heart\",\"thought\":\"嗨，休息一下吧\"}".into();
        let plan2 = client
            .plan_activity("sk-test-secret-1234", "deepseek-reasoner", &input)
            .await
            .unwrap();
        assert_eq!(plan2.action, ActivityAction::Wave);
        assert_eq!(plan2.thought.as_deref(), Some("嗨，休息一下吧"));

        // 3. 401 标记为 fatal_auth 且绝不回显 API Key
        *mock.next_status.lock().unwrap() = 401;
        let err = client
            .test_connection("sk-test-secret-1234", "deepseek-chat")
            .await
            .unwrap_err();
        assert!(err.fatal_auth);
        assert!(err.message.contains("401"));
        assert!(!err.message.contains("sk-test-secret-1234"));

        // 验证发送的请求格式
        let captured = mock.captured.lock().unwrap();
        assert_eq!(captured.len(), 3);
        assert_eq!(captured[0].0, "Bearer sk-test-secret-1234");
        assert_eq!(captured[0].1["model"], "deepseek-chat");
        assert_eq!(captured[0].1["response_format"]["type"], "json_object");
        assert!(captured[0].1.get("temperature").is_some());

        assert_eq!(captured[1].1["model"], "deepseek-reasoner");
        assert!(captured[1].1.get("response_format").is_none());
        assert!(captured[1].1.get("temperature").is_none());

        server.abort();
    }
}
