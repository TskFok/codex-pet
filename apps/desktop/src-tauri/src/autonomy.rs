//! DeepSeek API 驱动的桌宠桌面自由活动模块。
//!
//! 本模块严格限定仅用于桌宠的自主活动决策（漫步、巡逻、跳跃、招手、休息、张望及短句自言自语），
//! 不接触也不向外部发送任何会话内容、代码、项目名或系统通知详情。

use crate::{
    NotificationRuntime, notification_paths,
    placement::{DesktopContext, WindowPlacement},
    platform,
    settings::{
        ActivityRange, DEEPSEEK_DEFAULT_MODEL, DEFAULT_PET_NAME, MAX_ACTIVITY_SPEED,
        MIN_ACTIVITY_SPEED, Preferences, builtin_pet_name, valid_deepseek_model,
    },
};
use pet_core::{AppPaths, paths::wall_clock_ms};
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
pub const WALK_SPEED_PX_PER_SEC: f64 = 175.0;
const MIN_WALK_DURATION_MS: u64 = 900;
const MAX_WALK_DURATION_MS: u64 = 4_200;
const BASE_HOP_DURATION_MS: f64 = 760.0;
const MIN_HOP_DURATION_MS: f64 = 450.0;
const MAX_HOP_DURATION_MS: f64 = 1_200.0;
const MAX_THOUGHT_CHARS: usize = 64;
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
    pub activity_range: ActivityRange,
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
        let reply =
            sanitize_thought(&content).unwrap_or_else(|| "连接成功，随时可以自由活动！".into());
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

        let payload: ChatCompletionResponse = response.json().await.map_err(|_| DeepSeekError {
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
- action: 字符串，必须是 \"walk\" | \"patrol\" | \"hop\" | \"wave\" | \"rest\" | \"look_around\" 之一；优先在允许活动范围内多自由走动（walk/patrol/hop）并穿插原地动作。\n\
- targetX: 0.00 到 1.00 之间的小数，表示目标横向位置比例（0 在桌面最左，1 在最右），请保持在用户给出的允许活动范围 x 区间内。\n\
- targetY: 0.00 到 1.00 之间的小数，表示目标纵向位置比例（0 在桌面最上，1 在最下），请保持在用户给出的允许活动范围 y 区间内。\n\
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
    let range = input.activity_range.normalized();
    format!(
        "宠物名称：{}\n桌面活动区域：{}x{}（逻辑像素）\n允许活动范围：{}（x={:.2}..{:.2}, y={:.2}..{:.2}）\n当前位置比例：x={:.2}, y={:.2}\n已自由活动时长：{} 分钟\n最近活动记录：{}\n请规划下一次桌宠自由活动，targetX 与 targetY 需落在允许活动范围内，避免反复停留在相同位置或重复同一句话。",
        input.pet_name,
        input.work_area_width,
        input.work_area_height,
        range.preset.label_zh(),
        range.min_x.clamp(0.0, 1.0),
        range.max_x.clamp(0.0, 1.0),
        range.min_y.clamp(0.0, 1.0),
        range.max_y.clamp(0.0, 1.0),
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
    match raw
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
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
        return (Preferences::default(), DEFAULT_PET_NAME.into());
    };
    let prefs = Preferences::load(&paths).unwrap_or_default();
    let name = pet_name(&paths, &prefs.pet_id);
    (prefs, name)
}

fn pet_name(paths: &AppPaths, id: &str) -> String {
    if let Some(name) = builtin_pet_name(id) {
        name.into()
    } else {
        pet_core::pet_package::describe_pet(&paths.pets.join(id))
            .map(|manifest| manifest.name)
            .unwrap_or_else(|_| "桌宠".into())
    }
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

fn thought_duration_ms(thought: Option<&str>, base_ms: u64) -> Option<u64> {
    let chars = thought?.chars().count() as u64;
    let extra = chars.saturating_sub(14) * 80;
    Some((base_ms + extra).clamp(3_200, 6_500))
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

pub fn range_bounds_px(ctx: &DesktopContext, range: &ActivityRange) -> (f64, f64, f64, f64) {
    let range = range.normalized();
    let min_x = (range.min_x.clamp(0.0, 1.0) * ctx.max_x).clamp(0.0, ctx.max_x);
    let max_x = (range.max_x.clamp(0.0, 1.0) * ctx.max_x).clamp(min_x, ctx.max_x);
    let min_y = (range.min_y.clamp(0.0, 1.0) * ctx.max_y).clamp(0.0, ctx.max_y);
    let max_y = (range.max_y.clamp(0.0, 1.0) * ctx.max_y).clamp(min_y, ctx.max_y);
    (min_x, max_x, min_y, max_y)
}

pub fn resolve_walk_target(
    ctx: &DesktopContext,
    range: &ActivityRange,
    target_x: f64,
    target_y: f64,
) -> (f64, f64) {
    let (min_x, max_x, min_y, max_y) = range_bounds_px(ctx, range);
    let start_x = ctx.placement.x;
    let start_y = ctx.placement.y;
    let mut end_x = (target_x * ctx.max_x).clamp(min_x, max_x);
    let mut end_y = (target_y * ctx.max_y).clamp(min_y, max_y);
    let dist = (end_x - start_x).hypot(end_y - start_y);
    if dist < 48.0 {
        let span_x = max_x - min_x;
        let span_y = max_y - min_y;
        if span_x > 96.0 {
            let shift = (span_x * 0.25).clamp(56.0, 260.0);
            let base_x = start_x.clamp(min_x, max_x);
            end_x = if base_x + shift <= max_x {
                base_x + shift
            } else {
                (base_x - shift).max(min_x)
            };
        } else if span_y > 96.0 {
            let shift = (span_y * 0.25).clamp(56.0, 260.0);
            let base_y = start_y.clamp(min_y, max_y);
            end_y = if base_y + shift <= max_y {
                base_y + shift
            } else {
                (base_y - shift).max(min_y)
            };
        }
    }
    (end_x.clamp(min_x, max_x), end_y.clamp(min_y, max_y))
}

pub fn resolve_hop_target(
    ctx: &DesktopContext,
    range: &ActivityRange,
    target_x: f64,
    target_y: f64,
) -> (f64, f64) {
    let (min_x, max_x, min_y, max_y) = range_bounds_px(ctx, range);
    let start_x = ctx.placement.x;
    let start_y = ctx.placement.y;
    let desired_x = (target_x * ctx.max_x).clamp(min_x, max_x);
    let desired_y = (target_y * ctx.max_y).clamp(min_y, max_y);
    let raw_dx = (desired_x - start_x).clamp(-200.0, 200.0);
    let raw_dy = (desired_y - start_y).clamp(-120.0, 120.0);
    (
        (start_x + raw_dx).clamp(min_x, max_x),
        (start_y + raw_dy).clamp(min_y, max_y),
    )
}

pub fn compute_walk_duration_ms(dist_px: f64, speed_multiplier: f64) -> u64 {
    let multiplier = if speed_multiplier.is_finite() {
        speed_multiplier.clamp(MIN_ACTIVITY_SPEED, MAX_ACTIVITY_SPEED)
    } else {
        1.0
    };
    let speed_px_per_sec = WALK_SPEED_PX_PER_SEC * multiplier;
    let min_ms = (MIN_WALK_DURATION_MS as f64 / multiplier).round();
    let max_ms = (MAX_WALK_DURATION_MS as f64 / multiplier).round();
    ((dist_px.max(0.0) / speed_px_per_sec) * 1000.0)
        .clamp(min_ms, max_ms)
        .round() as u64
}

pub fn compute_hop_duration_ms(speed_multiplier: f64) -> u64 {
    let multiplier = if speed_multiplier.is_finite() {
        speed_multiplier.clamp(MIN_ACTIVITY_SPEED, MAX_ACTIVITY_SPEED)
    } else {
        1.0
    };
    (BASE_HOP_DURATION_MS / multiplier)
        .clamp(MIN_HOP_DURATION_MS, MAX_HOP_DURATION_MS)
        .round() as u64
}

#[allow(clippy::too_many_arguments)]
async fn execute_plan(
    app: &tauri::AppHandle,
    state: &AutonomyRuntime,
    ctx: &DesktopContext,
    range: &ActivityRange,
    speed_multiplier: f64,
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
            let (end_x, end_y) = resolve_walk_target(ctx, range, plan.target_x, plan.target_y);
            let dist = (end_x - start_x).hypot(end_y - start_y);
            let run_state = if end_x >= start_x {
                "running-right"
            } else {
                "running-left"
            };
            let walk_ms = compute_walk_duration_ms(dist, speed_multiplier);

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
                    thought_duration_ms: thought_duration_ms(plan.thought.as_deref(), 3_800),
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
            let (end_x, end_y) = resolve_hop_target(ctx, range, plan.target_x, plan.target_y);
            let hop_ms = compute_hop_duration_ms(speed_multiplier);

            emit_activity(
                app,
                &PetActivityEvent {
                    id: activity_id.to_owned(),
                    phase: "start",
                    state: Some("jumping"),
                    effect: effect_payload(plan.effect.as_deref().or(Some("star")), 1.05),
                    thought: plan.thought.clone(),
                    thought_duration_ms: thought_duration_ms(plan.thought.as_deref(), 3_400),
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
                ActivityAction::LookAround => (
                    "review",
                    Some("sparkle"),
                    plan.duration_ms.clamp(2_000, 4_500),
                ),
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
                    thought_duration_ms: thought_duration_ms(plan.thought.as_deref(), 3_600),
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
            activity_range: prefs.activity_range.clone(),
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
                if !latest_prefs.deepseek_enabled || latest_prefs.deepseek_api_key.trim().is_empty()
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
                        &latest_prefs.activity_range,
                        latest_prefs.activity_speed,
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
        use crate::settings::ActivityRangePreset;
        let prompt = build_user_prompt(&ActivityPromptInput {
            pet_name: DEFAULT_PET_NAME.into(),
            work_area_width: 1440,
            work_area_height: 900,
            current_norm_x: 0.25,
            current_norm_y: 0.75,
            activity_range: ActivityRange::from_preset(ActivityRangePreset::Bottom),
            active_minutes: 12,
            recent_actions: vec!["walk(0.25,0.75)".into()],
        });
        assert!(prompt.contains(DEFAULT_PET_NAME));
        assert!(prompt.contains("1440x900"));
        assert!(prompt.contains("屏幕底部"));
        assert!(prompt.contains("x=0.00..1.00, y=0.70..1.00"));
        assert!(prompt.contains("x=0.25, y=0.75"));
        assert!(prompt.contains("walk(0.25,0.75)"));
    }

    #[test]
    fn activity_prompt_uses_builtin_whale_maid_name_without_an_import() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(dir.path());
        let prompt = build_user_prompt(&ActivityPromptInput {
            pet_name: pet_name(&paths, "deepseek-whale-maid"),
            work_area_width: 1440,
            work_area_height: 900,
            current_norm_x: 0.5,
            current_norm_y: 0.5,
            activity_range: ActivityRange::default(),
            active_minutes: 0,
            recent_actions: Vec::new(),
        });
        assert!(prompt.contains("DeepSeek娘"));
    }

    #[test]
    fn walk_and_hop_targets_respect_configured_activity_range() {
        use crate::settings::ActivityRangePreset;
        let ctx = DesktopContext {
            placement: WindowPlacement {
                x: 600.0,
                y: 500.0,
                scale: 1.0,
                monitor_id: Some("main".into()),
            },
            work_area_width: 1440.0,
            work_area_height: 900.0,
            max_x: 1200.0,
            max_y: 580.0,
        };

        let bottom = ActivityRange::from_preset(ActivityRangePreset::Bottom);
        let (x, y) = resolve_walk_target(&ctx, &bottom, 0.2, 0.1);
        assert!((x - 240.0).abs() < 1e-6);
        assert!((y - 580.0 * 0.7).abs() < 1e-6);

        let right_edge = ActivityRange::from_preset(ActivityRangePreset::RightEdge);
        let (rx, ry) = resolve_walk_target(&ctx, &right_edge, 0.1, 0.4);
        assert!((rx - 1200.0 * 0.75).abs() < 1e-6);
        assert!((ry - 580.0 * 0.4).abs() < 1e-6);

        let custom = ActivityRange {
            preset: ActivityRangePreset::Custom,
            min_x: 0.2,
            max_x: 0.4,
            min_y: 0.5,
            max_y: 0.8,
        };
        let (hx, hy) = resolve_hop_target(&ctx, &custom, 0.9, 0.1);
        assert!((240.0..=480.0).contains(&hx));
        assert!((290.0..=464.0).contains(&hy));
    }

    #[test]
    fn walk_and_hop_durations_scale_with_activity_speed_and_bounds() {
        // 基准 1.0x：525px / 175px/s = 3000ms
        assert_eq!(compute_walk_duration_ms(525.0, 1.0), 3_000);
        // 0.5x 慢速：速度减半，时长翻倍，且上限随之放宽到 8400ms
        assert_eq!(compute_walk_duration_ms(525.0, 0.5), 6_000);
        assert_eq!(compute_walk_duration_ms(2000.0, 0.5), 8_400);
        assert_eq!(compute_walk_duration_ms(10.0, 0.5), 1_800);
        // 2.0x 快速：速度翻倍，时长减半，上下限同步缩放到 450..=2100ms
        assert_eq!(compute_walk_duration_ms(525.0, 2.0), 1_500);
        assert_eq!(compute_walk_duration_ms(2000.0, 2.0), 2_100);
        assert_eq!(compute_walk_duration_ms(10.0, 2.0), 450);

        // 蹦跳时长随速率联动且在合理上下限内
        assert_eq!(compute_hop_duration_ms(1.0), 760);
        assert_eq!(compute_hop_duration_ms(0.5), 1_200);
        assert_eq!(compute_hop_duration_ms(2.0), 450);
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
            pet_name: DEFAULT_PET_NAME.into(),
            work_area_width: 1440,
            work_area_height: 900,
            current_norm_x: 0.4,
            current_norm_y: 0.7,
            activity_range: ActivityRange::default(),
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

    #[test]
    fn thought_sanitization_keeps_longer_sentences_and_scales_reading_duration() {
        let forty_five =
            "今天桌面好安静呀，我去右边巡逻一圈顺便伸个大大的懒腰，等你忙完随时可以叫我一起玩哦~";
        assert!(forty_five.chars().count() > 36 && forty_five.chars().count() <= MAX_THOUGHT_CHARS);
        assert_eq!(sanitize_thought(forty_five).as_deref(), Some(forty_five));

        let too_long: String = "喵".repeat(MAX_THOUGHT_CHARS + 10);
        let sanitized = sanitize_thought(&too_long).unwrap();
        assert_eq!(sanitized.chars().count(), MAX_THOUGHT_CHARS + 1);
        assert!(sanitized.ends_with('…'));

        assert_eq!(thought_duration_ms(None, 3_600), None);
        assert_eq!(thought_duration_ms(Some("出去逛逛~"), 3_600), Some(3_600));
        let long_ms = thought_duration_ms(Some(forty_five), 3_600).unwrap();
        assert!(long_ms > 3_600 && long_ms <= 6_500);
    }
}
