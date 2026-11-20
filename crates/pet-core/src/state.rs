//! 可重放的状态归约器。时间戳仅用于展示和历史保留，不作为回合因果依据。
use crate::protocol::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

const MAX_HISTORY: usize = 1_000;
const HISTORY_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplyResult {
    /// 公开快照是否改变；不代表是否需要持久化接收事件或去重记录。
    pub changed: bool,
    /// 单次实时新增提醒；离线恢复应汇总最终快照后展示。
    pub should_notify: bool,
}

#[derive(Serialize, Deserialize)]
struct SessionEntry {
    projection: SessionProjection,
    instance_id: String,
    sequence: Option<u64>,
    known_turns: HashSet<Option<String>>,
    terminal: bool,
    terminal_status: Option<SessionStatus>,
    // 独立跨回合请求不提供当前运行回合的因果锚点。
    has_turn_fact: bool,
    // 无源seq且先于新回合开始送达的普通请求，等明确回合锚点后再应用。
    deferred_requests: Vec<EventEnvelope>,
    running_known: bool,
    external_unread: bool,
    // 导入的未读结束独立于当前回合及来源快照的 unread 值。
    external_unread_end: bool,
    #[serde(default)]
    observation: Option<ObservedGeneration>,
    #[serde(default)]
    uncertain_pending: HashSet<(Option<String>, String)>,
    #[serde(default)]
    timed_births: Vec<RecoveredQuestion>,
    #[serde(default)]
    direct_questions: Vec<DirectQuestionRef>,
    #[serde(default)]
    audit_only: bool,
}

#[derive(Serialize, Deserialize)]
struct ObservedGeneration {
    proof: ObservationGeneration,
    snapshot_event_count: Option<u64>,
    timed_event_count: Option<u64>,
    confirmed_ends: HashSet<String>,
    #[serde(default)]
    snapshot_running: Option<bool>,
    #[serde(default)]
    last_terminal_sequence: Option<u64>,
}

#[derive(Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
struct DirectQuestionRef {
    owner: String,
    epoch: String,
    turn: Option<String>,
    request: String,
}

impl DirectQuestionRef {
    fn from_event(event: &EventEnvelope, request_id: &str) -> Option<Self> {
        Some(Self {
            owner: event.direct_question_owner.clone()?,
            epoch: event.observation.as_ref()?.epoch_id.clone(),
            turn: event.turn_id.clone(),
            request: request_id.into(),
        })
    }

    fn matches_pending(&self, pending: &PendingRequest) -> bool {
        self.turn == pending.turn_id && self.request == pending.id
    }
}

impl SessionEntry {
    fn new(event: &EventEnvelope) -> Self {
        Self {
            projection: SessionProjection {
                key: event.session_key(),
                parent_session_id: event.parent_session_id.clone(),
                turn_id: event.turn_id.clone(),
                status: SessionStatus::Unknown,
                pending: vec![],
                project: event.project.clone(),
                summary: event.summary.clone(),
                unread: false,
                last_event_at_ms: event.occurred_at_ms,
            },
            instance_id: event.identity.instance_id.clone(),
            sequence: None,
            // 请求的 birth turn 不是已观察到的活动回合。
            known_turns: HashSet::new(),
            terminal: false,
            terminal_status: None,
            has_turn_fact: false,
            deferred_requests: vec![],
            running_known: false,
            external_unread: false,
            external_unread_end: false,
            observation: None,
            uncertain_pending: HashSet::new(),
            timed_births: Vec::new(),
            direct_questions: Vec::new(),
            audit_only: false,
        }
    }

    fn confirmed_pending(&self) -> bool {
        self.projection
            .pending
            .iter()
            .any(|request| self.pending_is_confirmed(request))
    }

    fn pending_is_confirmed(&self, request: &PendingRequest) -> bool {
        !self
            .uncertain_pending
            .contains(&(request.turn_id.clone(), request.id.clone()))
    }

    fn comparable_sequence(&self, event: &EventEnvelope) -> Option<std::cmp::Ordering> {
        if self.instance_id != event.identity.instance_id {
            return None;
        }
        Some(event.sequence?.cmp(&self.sequence?))
    }

    fn observe_sequence(&mut self, event: &EventEnvelope) {
        if self.instance_id == event.identity.instance_id {
            self.sequence = self.sequence.max(event.sequence);
        } else {
            self.instance_id.clone_from(&event.identity.instance_id);
            self.sequence = event.sequence;
        }
    }

    fn take_current_deferred_requests(&mut self) -> Vec<EventEnvelope> {
        let mut deferred = vec![];
        self.deferred_requests.retain(|pending| {
            if pending.turn_id == self.projection.turn_id {
                deferred.push(pending.clone());
                false
            } else {
                true
            }
        });
        deferred
    }
}

#[derive(Default)]
pub struct SessionStore {
    revision: u64,
    sessions: BTreeMap<SessionKey, SessionEntry>,
    notifications: Vec<NotificationRecord>,
    // 直接使用归约器时在内存去重，日志使用本地接收时间索引。
    seen_events: HashSet<String>,
    resolved: HashSet<(SessionKey, Option<String>, String)>,
    // 明确回合的结束事实独立于当前投影和可见历史，裁剪历史不能使它复活。
    confirmed_turn_ends: HashSet<(SessionKey, String)>,
    scoped_resolved: HashSet<(SessionKey, String, Option<String>, String)>,
    direct_resolved: HashSet<(SessionKey, DirectQuestionRef)>,
    notification_epochs: BTreeMap<String, String>,
    now_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "SessionStore")]
pub(crate) struct SessionCheckpoint {
    revision: u64,
    #[serde(with = "session_entries")]
    sessions: BTreeMap<SessionKey, SessionEntry>,
    notifications: Vec<NotificationRecord>,
    #[serde(skip)]
    seen_events: HashSet<String>,
    resolved: HashSet<(SessionKey, Option<String>, String)>,
    confirmed_turn_ends: HashSet<(SessionKey, String)>,
    #[serde(default)]
    scoped_resolved: HashSet<(SessionKey, String, Option<String>, String)>,
    #[serde(default)]
    direct_resolved: HashSet<(SessionKey, DirectQuestionRef)>,
    #[serde(default)]
    notification_epochs: BTreeMap<String, String>,
    now_ms: u64,
}

impl SessionStore {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn time(&self) -> u64 {
        self.now_ms
    }

    /// 调用者必须先在跨进程入口通过 `decode_event` 验证。
    pub fn apply(&mut self, event: &EventEnvelope) -> ApplyResult {
        if !self.seen_events.insert(event.event_id.clone()) {
            return ApplyResult::default();
        }
        self.apply_event(event)
    }

    pub(crate) fn apply_event(&mut self, event: &EventEnvelope) -> ApplyResult {
        let before = self.snapshot();
        self.now_ms = self
            .now_ms
            .max(event.received_at_ms.unwrap_or(event.occurred_at_ms));
        let accepted = self.accept_observation(event);
        let should_notify = if let EventKind::AttentionResolved { request_id } = &event.kind
            && let Some(question) = DirectQuestionRef::from_event(event, request_id)
        {
            self.resolve_direct(event, question, accepted);
            false
        } else if !accepted {
            return ApplyResult::default();
        } else if let Some(notify) = self.apply_idle_turn_end(event) {
            notify
        } else {
            match &event.kind {
                EventKind::SourceDisconnected => {
                    for entry in self.sessions.values_mut().filter(|entry| {
                        entry.observation.is_none()
                            && entry.projection.key.source == event.identity.source
                            && entry.projection.key.installation_id
                                == event.identity.installation_id
                            && entry.instance_id == event.identity.instance_id
                    }) {
                        if !entry.terminal {
                            mark_activity_unknown(entry);
                        }
                    }
                    false
                }
                EventKind::SourceSnapshot { sessions } => {
                    self.apply_source_snapshot(event, sessions);
                    false
                }
                EventKind::SourceRecovery {
                    snapshot_event_count,
                    running,
                    timed,
                } => {
                    self.apply_recovery_snapshot(
                        event,
                        *snapshot_event_count,
                        *running,
                        timed.as_deref(),
                    );
                    false
                }
                _ => self.apply_session_event(event),
            }
        };
        self.prune_history();
        self.refresh_unread();
        let changed = before != self.snapshot();
        if changed {
            self.revision += 1;
        }
        ApplyResult {
            changed,
            should_notify,
        }
    }

    fn accept_observation(&mut self, event: &EventEnvelope) -> bool {
        let key = event.session_key();
        let current = self
            .sessions
            .get(&key)
            .and_then(|entry| entry.observation.as_ref());
        let Some(proof) = &event.observation else {
            return current.is_none();
        };
        if let Some(current) = current {
            if current.proof.stream_id != proof.stream_id
                || proof.generation < current.proof.generation
            {
                return false;
            }
            if proof.generation == current.proof.generation {
                if *proof != current.proof {
                    return false;
                }
                let below_snapshot = event
                    .sequence
                    .is_some_and(|seq| current.snapshot_event_count.is_some_and(|cut| seq < cut));
                return !below_snapshot
                    || match event.kind {
                        EventKind::TurnStarted => false,
                        EventKind::TurnEnded { .. } => current.snapshot_running == Some(false),
                        _ => true,
                    };
            }
            if !proof.ancestors.iter().any(|old| {
                old.generation == current.proof.generation
                    && old.epoch_id == current.proof.epoch_id
                    && old.first_sequence == current.proof.recovered_event_count
            }) {
                return false;
            }
        }
        let mut entry = SessionEntry::new(event);
        if let Some(previous) = self.sessions.remove(&key) {
            entry.audit_only = previous.audit_only;
            entry.external_unread = previous.external_unread;
            entry.external_unread_end = previous.external_unread_end;
            entry.projection.pending = previous.projection.pending;
            entry.timed_births = previous.timed_births;
            entry.direct_questions = previous.direct_questions;
            entry.projection.pending.retain(|pending| {
                pending.lifetime == RequestLifetime::UntilResolved
                    && !entry.timed_births.iter().any(|timed| {
                        timed.turn_id == pending.turn_id
                            && timed.request_id == pending.id
                            && timed.birth_sequence >= proof.live_start()
                    })
            });
            entry
                .timed_births
                .retain(|timed| timed.birth_sequence < proof.live_start());
            entry.uncertain_pending.extend(
                entry
                    .projection
                    .pending
                    .iter()
                    .map(|pending| (pending.turn_id.clone(), pending.id.clone())),
            );
        } else {
            entry.audit_only = event.sequence.is_none()
                && matches!(event.kind, EventKind::AttentionResolved { .. });
        }
        entry.observation = Some(ObservedGeneration {
            proof: proof.clone(),
            snapshot_event_count: None,
            timed_event_count: None,
            confirmed_ends: HashSet::new(),
            snapshot_running: None,
            last_terminal_sequence: None,
        });
        self.sessions.insert(key, entry);
        true
    }

    fn apply_recovery_snapshot(
        &mut self,
        event: &EventEnvelope,
        cut: u64,
        running: bool,
        timed: Option<&[RecoveredQuestion]>,
    ) {
        let key = event.session_key();
        let Some(mut entry) = self.sessions.remove(&key) else {
            return;
        };
        entry.audit_only = false;
        let generation = entry.observation.as_mut().expect("恢复快照已有来源证明");
        if generation
            .snapshot_event_count
            .is_none_or(|previous| cut > previous)
            && entry.sequence.is_none_or(|seq| seq < cut)
        {
            if running || !entry.terminal {
                entry.projection.turn_id.clone_from(&event.turn_id);
                entry.terminal = false;
                entry.terminal_status = None;
            }
            entry.running_known = running;
            entry.has_turn_fact = true;
            entry.known_turns.insert(entry.projection.turn_id.clone());
            entry.sequence = event.sequence;
        }
        if generation
            .snapshot_event_count
            .is_none_or(|previous| cut > previous)
        {
            generation.snapshot_event_count = Some(cut);
            generation.snapshot_running = Some(running);
        }
        if let Some(questions) = timed
            && generation
                .timed_event_count
                .is_none_or(|previous| cut > previous)
        {
            generation.timed_event_count = Some(cut);
            entry.projection.pending.retain(|pending| {
                !entry.timed_births.iter().any(|old| {
                    old.birth_sequence < cut
                        && generation.proof.covers_birth(old.birth_sequence)
                        && old.turn_id == pending.turn_id
                        && old.request_id == pending.id
                        && !questions.contains(old)
                })
            });
            entry.timed_births.retain(|old| {
                old.birth_sequence >= cut || !generation.proof.covers_birth(old.birth_sequence)
            });
            for question in questions {
                if self.is_resolved(
                    &key,
                    &question.turn_id,
                    &question.request_id,
                    event
                        .observation
                        .as_ref()
                        .map(|proof| proof.epoch_id.as_str()),
                ) {
                    continue;
                }
                entry
                    .uncertain_pending
                    .remove(&(question.turn_id.clone(), question.request_id.clone()));
                if !entry.projection.pending.iter().any(|pending| {
                    pending.turn_id == question.turn_id && pending.id == question.request_id
                }) {
                    entry.projection.pending.push(PendingRequest {
                        id: question.request_id.clone(),
                        turn_id: question.turn_id.clone(),
                        reason: AttentionReason::Answer,
                        summary: None,
                        lifetime: RequestLifetime::UntilResolved,
                    });
                }
                entry.timed_births.push(question.clone());
            }
        }
        restore_entry_status(&mut entry);
        self.sessions.insert(key, entry);
    }

    fn is_resolved(
        &self,
        key: &SessionKey,
        turn: &Option<String>,
        request_id: &str,
        epoch: Option<&str>,
    ) -> bool {
        if let Some(epoch) = epoch {
            self.scoped_resolved.contains(&(
                key.clone(),
                epoch.into(),
                turn.clone(),
                request_id.into(),
            ))
        } else {
            self.resolved
                .contains(&(key.clone(), turn.clone(), request_id.into()))
        }
    }

    fn resolve_direct(
        &mut self,
        event: &EventEnvelope,
        question: DirectQuestionRef,
        accepted: bool,
    ) {
        let key = event.session_key();
        let Some(entry) = self.sessions.get_mut(&key) else {
            return;
        };
        let exact = entry.direct_questions.contains(&question);
        if !accepted {
            let Some(current) = &entry.observation else {
                return;
            };
            let Some(old) = &event.observation else {
                return;
            };
            if !exact
                || old.stream_id != current.proof.stream_id
                || !current.proof.ancestors.iter().any(|ancestor| {
                    ancestor.epoch_id == old.epoch_id
                        && ancestor.generation == old.generation
                        && ancestor.first_sequence == old.recovered_event_count
                })
            {
                return;
            }
        }
        if exact {
            entry
                .projection
                .pending
                .retain(|pending| !question.matches_pending(pending));
            entry
                .uncertain_pending
                .remove(&(question.turn.clone(), question.request.clone()));
            entry
                .direct_questions
                .retain(|previous| previous != &question);
            restore_entry_status(entry);
            self.resolve_notifications(
                &key,
                &question.turn,
                Some(&question.request),
                Some(&question.epoch),
            );
        }
        self.direct_resolved.insert((key, question));
    }

    fn apply_idle_turn_end(&mut self, event: &EventEnvelope) -> Option<bool> {
        let EventKind::TurnEnded { outcome } = &event.kind else {
            return None;
        };
        let (turn, seq) = (event.turn_id.as_ref()?, event.sequence?);
        let key = event.session_key();
        let entry = self.sessions.get_mut(&key)?;
        let generation = entry.observation.as_mut()?;
        let cut = generation.snapshot_event_count?;
        if generation.snapshot_running != Some(false) || seq >= cut {
            return None;
        }
        if !generation.confirmed_ends.insert(turn.clone()) {
            return Some(false);
        }
        entry.projection.pending.retain(|pending| {
            pending.lifetime == RequestLifetime::UntilResolved || pending.turn_id != event.turn_id
        });
        if !entry.running_known
            && entry.sequence.is_none_or(|latest| latest < cut)
            && generation
                .last_terminal_sequence
                .is_none_or(|last| seq >= last)
        {
            entry.projection.turn_id.clone_from(&event.turn_id);
            entry.terminal = true;
            entry.terminal_status = Some(status_for_outcome(*outcome));
            generation.last_terminal_sequence = Some(seq);
            entry.known_turns.insert(event.turn_id.clone());
        }
        let epoch = generation.proof.epoch_id.clone();
        restore_entry_status(entry);
        self.resolve_notifications(&key, &event.turn_id, None, Some(&epoch));
        self.push_notification(event, NotificationKind::TurnEnded { outcome: *outcome });
        Some(true)
    }

    fn apply_session_event(&mut self, event: &EventEnvelope) -> bool {
        let key = event.session_key();
        let epoch = event
            .observation
            .as_ref()
            .map(|proof| proof.epoch_id.as_str());
        let scope = event
            .observation
            .as_ref()
            .and_then(|proof| proof.request_epoch(event.question_birth));
        if let EventKind::AttentionResolved { request_id } = &event.kind {
            if let Some(epoch) = epoch {
                self.scoped_resolved.insert((
                    key.clone(),
                    epoch.into(),
                    event.turn_id.clone(),
                    request_id.clone(),
                ));
            } else {
                self.resolved
                    .insert((key.clone(), event.turn_id.clone(), request_id.clone()));
            }
        }
        let existing = self.sessions.remove(&key);
        if existing.as_ref().is_none_or(|entry| entry.audit_only)
            && event.identity.source == Source::Deepseek
            && event.sequence.is_none()
        {
            let audit_only = match &event.kind {
                EventKind::AttentionResolved { .. } => true,
                EventKind::AttentionRequested { request_id, .. } => {
                    self.is_resolved(&key, &event.turn_id, request_id, epoch)
                }
                _ => false,
            };
            if audit_only {
                if let Some(entry) = existing {
                    self.sessions.insert(key, entry);
                }
                return false;
            }
        }
        let awaiting_causal_order = existing.as_ref().is_some_and(|entry| {
            (entry.observation.is_none() || entry.has_turn_fact)
                && (entry.projection.status == SessionStatus::Unknown
                    || (!entry.running_known
                        && entry.terminal_status.is_none()
                        && entry.has_turn_fact))
        });
        let mut entry = existing.unwrap_or_else(|| SessionEntry::new(event));
        let order = entry.comparable_sequence(event);
        let different_turn = entry.projection.turn_id != event.turn_id;
        // 跨回合请求独立于运行回合；显式解除先于终态/旧回合守卫处理。
        if let EventKind::AttentionResolved { request_id } = &event.kind
            && (different_turn
                || entry.projection.pending.iter().any(|pending| {
                    pending.id == *request_id
                        && pending.turn_id == event.turn_id
                        && pending.lifetime == RequestLifetime::UntilResolved
                }))
        {
            entry
                .projection
                .pending
                .retain(|pending| !(pending.id == *request_id && pending.turn_id == event.turn_id));
            entry.deferred_requests.retain(|pending| {
                !matches!(&pending.kind,
                EventKind::AttentionRequested { request_id: id, .. }
                    if id == request_id && pending.turn_id == event.turn_id)
            });
            self.resolve_notifications(&key, &event.turn_id, Some(request_id), scope);
            restore_entry_status(&mut entry);
            self.sessions.insert(key, entry);
            return false;
        }
        if let EventKind::AttentionRequested {
            request_id,
            reason,
            lifetime: RequestLifetime::UntilResolved,
        } = &event.kind
        {
            let notify = self.add_attention(
                &mut entry,
                event,
                request_id,
                *reason,
                RequestLifetime::UntilResolved,
            );
            entry.projection.last_event_at_ms =
                entry.projection.last_event_at_ms.max(event.occurred_at_ms);
            self.sessions.insert(key, entry);
            return notify;
        }
        if event.turn_id.as_ref().is_some_and(|turn| {
            if let Some(generation) = &entry.observation {
                generation.confirmed_ends.contains(turn)
            } else {
                self.confirmed_turn_ends
                    .contains(&(key.clone(), turn.clone()))
            }
        }) && matches!(
            event.kind,
            EventKind::TurnStarted
                | EventKind::AttentionRequested { .. }
                | EventKind::TurnEnded { .. }
        ) {
            self.sessions.insert(key, entry);
            return false;
        }
        // 无法判断哪个回合较新，不影响“这个明确回合已经结束”的独立事实。
        // 同回合低水位结束继续受旧保护，不能清掉较新请求；无 ID 周期也不建全局墓碑。
        let mut end_fact_notify = false;
        if let (Some(turn), EventKind::TurnEnded { outcome }) = (&event.turn_id, &event.kind)
            && (different_turn || !order.is_some_and(|ordering| ordering.is_lt()))
        {
            if let Some(generation) = &mut entry.observation {
                generation.confirmed_ends.insert(turn.clone());
            } else {
                self.confirmed_turn_ends.insert((key.clone(), turn.clone()));
            }
            entry.projection.pending.retain(|pending| {
                pending.lifetime == RequestLifetime::UntilResolved
                    || pending.turn_id != event.turn_id
            });
            self.resolve_notifications(&key, &event.turn_id, None, scope);
            self.push_notification(event, NotificationKind::TurnEnded { outcome: *outcome });
            end_fact_notify = true;
        }
        if !different_turn
            && matches!(event.kind, EventKind::TurnStarted)
            && (order.is_some_and(|order| !order.is_gt())
                || (awaiting_causal_order && order.is_none()))
        {
            self.sessions.insert(key, entry);
            return false;
        }
        if different_turn {
            if entry.known_turns.contains(&event.turn_id) || order.is_some_and(|o| o.is_lt()) {
                entry.known_turns.insert(event.turn_id.clone());
                self.sessions.insert(key, entry);
                return end_fact_notify;
            }
            if order != Some(std::cmp::Ordering::Greater)
                && !(matches!(event.kind, EventKind::TurnStarted)
                    && event.sequence.is_some()
                    && !entry.has_turn_fact)
            {
                if matches!(event.kind, EventKind::AttentionRequested { .. }) {
                    entry.deferred_requests.push(event.clone());
                }
                mark_activity_unknown(&mut entry);
                self.sessions.insert(key, entry);
                return end_fact_notify;
            }
            self.resolve_notifications(&key, &entry.projection.turn_id, None, scope);
            entry
                .projection
                .pending
                .retain(|pending| pending.lifetime == RequestLifetime::UntilResolved);
            entry.projection.turn_id.clone_from(&event.turn_id);
            entry.known_turns.insert(event.turn_id.clone());
            entry.terminal = false;
            entry.terminal_status = None;
            entry.running_known = false;
        } else if entry.terminal {
            // 同一已结束回合不能被开始/请求重开；无回合 ID 时只接受可比较的新开始。
            if matches!(event.kind, EventKind::TurnStarted) && event.turn_id.is_none() {
                if order == Some(std::cmp::Ordering::Greater) {
                    entry.terminal = false;
                    entry.terminal_status = None;
                    entry.running_known = false;
                } else if order.is_none() {
                    mark_activity_unknown(&mut entry);
                    self.sessions.insert(key, entry);
                    return false;
                } else {
                    self.sessions.insert(key, entry);
                    return false;
                }
            } else {
                self.sessions.insert(key, entry);
                return end_fact_notify;
            }
        }
        // 已知同一回合的多请求允许乱序；请求解除按精确关联而非会话水位处理。
        let notify = match &event.kind {
            EventKind::TurnStarted => {
                entry.running_known = true;
                restore_entry_status(&mut entry);
                false
            }
            EventKind::AttentionRequested {
                request_id,
                reason,
                lifetime,
            } => {
                let added = self.add_attention(&mut entry, event, request_id, *reason, *lifetime);
                if added {
                    entry.running_known = true;
                }
                added
            }
            EventKind::AttentionResolved { request_id } => {
                entry.projection.pending.retain(|pending| {
                    !(pending.id == *request_id && pending.turn_id == event.turn_id)
                });
                self.resolve_notifications(&key, &event.turn_id, Some(request_id), scope);
                restore_entry_status(&mut entry);
                false
            }
            EventKind::TurnEnded { outcome } => {
                // 低序号的结束不能结束尚在更新的同一回合。
                if order.is_some_and(|o| o.is_lt()) {
                    self.sessions.insert(key, entry);
                    return false;
                }
                entry.terminal = true;
                entry.terminal_status = Some(status_for_outcome(*outcome));
                if let Some(generation) = &mut entry.observation {
                    generation.last_terminal_sequence = event.sequence;
                }
                entry.projection.status = status_for_outcome(*outcome);
                entry
                    .projection
                    .pending
                    .retain(|pending| pending.lifetime == RequestLifetime::UntilResolved);
                self.resolve_notifications(&key, &event.turn_id, None, scope);
                restore_entry_status(&mut entry);
                if !end_fact_notify {
                    self.push_notification(
                        event,
                        NotificationKind::TurnEnded { outcome: *outcome },
                    );
                }
                true
            }
            EventKind::SourceDisconnected
            | EventKind::SourceSnapshot { .. }
            | EventKind::SourceRecovery { .. } => unreachable!(),
        };
        entry.audit_only = false;
        entry.has_turn_fact = true;
        entry.known_turns.insert(event.turn_id.clone());
        entry.observe_sequence(event);
        entry.projection.last_event_at_ms =
            entry.projection.last_event_at_ms.max(event.occurred_at_ms);
        if event.parent_session_id.is_some() {
            entry
                .projection
                .parent_session_id
                .clone_from(&event.parent_session_id);
        }
        if event.summary.is_some() {
            entry.projection.summary.clone_from(&event.summary);
        }
        if event.project.is_some() {
            entry.projection.project.clone_from(&event.project);
        }
        let deferred = entry.take_current_deferred_requests();
        self.sessions.insert(key, entry);
        let mut notified = notify;
        for request in deferred {
            notified |= self.apply_session_event(&request);
        }
        notified
    }

    fn add_attention(
        &mut self,
        entry: &mut SessionEntry,
        event: &EventEnvelope,
        request_id: &str,
        reason: AttentionReason,
        lifetime: RequestLifetime,
    ) -> bool {
        let key = event.session_key();
        let epoch = event
            .observation
            .as_ref()
            .map(|proof| proof.epoch_id.as_str());
        if self.is_resolved(&key, &event.turn_id, request_id, epoch) {
            return false;
        }
        if let Some(question) = DirectQuestionRef::from_event(event, request_id) {
            if self.direct_resolved.contains(&(key, question.clone())) {
                return false;
            }
            if !entry.direct_questions.contains(&question) {
                entry.direct_questions.retain(|previous| {
                    previous.turn != question.turn || previous.request != question.request
                });
                entry
                    .projection
                    .pending
                    .retain(|pending| !question.matches_pending(pending));
                entry.direct_questions.push(question);
            }
        }
        if let Some(birth_sequence) = event.question_birth {
            let fact = RecoveredQuestion {
                request_id: request_id.into(),
                turn_id: event.turn_id.clone(),
                birth_sequence,
            };
            let covered = entry
                .observation
                .as_ref()
                .and_then(|state| state.timed_event_count)
                .is_some_and(|cut| birth_sequence < cut);
            if covered && !entry.timed_births.contains(&fact) {
                return false;
            }
            if !entry.timed_births.contains(&fact) {
                entry.timed_births.push(fact);
            }
        }
        entry
            .uncertain_pending
            .remove(&(event.turn_id.clone(), request_id.into()));
        if entry
            .projection
            .pending
            .iter()
            .any(|pending| pending.id == request_id && pending.turn_id == event.turn_id)
        {
            restore_entry_status(entry);
            return false;
        }
        entry.audit_only = false;
        entry.projection.pending.push(PendingRequest {
            id: request_id.into(),
            turn_id: event.turn_id.clone(),
            reason,
            lifetime,
            summary: event.summary.clone(),
        });
        entry
            .projection
            .pending
            .sort_by(|a, b| (&a.turn_id, &a.id).cmp(&(&b.turn_id, &b.id)));
        entry.projection.status = SessionStatus::NeedsAttention;
        self.push_notification(
            event,
            NotificationKind::Attention {
                request_id: request_id.into(),
                reason,
                lifetime,
                resolved: false,
            },
        );
        true
    }

    fn apply_source_snapshot(&mut self, event: &EventEnvelope, projections: &[SessionProjection]) {
        let source_key = event.session_key();
        let included: HashSet<_> = projections.iter().map(|p| p.key.clone()).collect();
        for entry in self.sessions.values_mut().filter(|entry| {
            entry.observation.is_none()
                && entry.projection.key.source == event.identity.source
                && entry.projection.key.installation_id == event.identity.installation_id
                && entry.instance_id == event.identity.instance_id
                && !included.contains(&entry.projection.key)
                && (entry.projection.key != source_key
                    || entry
                        .comparable_sequence(event)
                        .is_none_or(|order| order.is_gt()))
        }) {
            mark_activity_unknown(entry);
        }
        for projection in projections {
            // apply 的调用者应先 decode_event；仍避免不可信快照越过来源边界。
            if projection.key.source != event.identity.source
                || projection.key.installation_id != event.identity.installation_id
            {
                continue;
            }
            if projection.turn_id.as_ref().is_some_and(|turn| {
                self.confirmed_turn_ends
                    .contains(&(projection.key.clone(), turn.clone()))
            }) {
                continue;
            }
            if let Some(existing) = self.sessions.get_mut(&projection.key) {
                if existing.observation.is_some() {
                    continue;
                }
                let ordering = existing.comparable_sequence(event);
                if (existing.projection.turn_id != projection.turn_id
                    && existing.known_turns.contains(&projection.turn_id))
                    || (existing.terminal && existing.projection.turn_id == projection.turn_id)
                {
                    continue;
                }
                if projection.key != source_key || ordering.is_none() {
                    if existing.projection != *projection {
                        mark_activity_unknown(existing);
                    }
                    continue;
                }
                if ordering != Some(std::cmp::Ordering::Greater) {
                    continue;
                }
                self.resolve_snapshot_requests(projection);
            }
            let mut entry = SessionEntry::new(event);
            entry.projection = projection.clone();
            if let Some(old) = self.sessions.get(&projection.key) {
                for request in old
                    .projection
                    .pending
                    .iter()
                    .filter(|request| request.lifetime == RequestLifetime::UntilResolved)
                {
                    if let Some(new) = entry
                        .projection
                        .pending
                        .iter_mut()
                        .find(|new| new.id == request.id && new.turn_id == request.turn_id)
                    {
                        // 普通快照没有精确解除事实，不能降级已证的持续请求。
                        *new = request.clone();
                    } else {
                        entry.projection.pending.push(request.clone());
                    }
                }
            }
            entry.projection.pending.retain(|request| {
                !self.resolved.contains(&(
                    projection.key.clone(),
                    request.turn_id.clone(),
                    request.id.clone(),
                ))
            });
            if entry.projection.pending.is_empty()
                && projection.status == SessionStatus::NeedsAttention
            {
                entry.projection.status = SessionStatus::Unknown;
            }
            if !entry.projection.pending.is_empty() {
                entry.projection.status = SessionStatus::NeedsAttention;
            }
            entry.has_turn_fact = true;
            entry.known_turns = HashSet::from([projection.turn_id.clone()]);
            if let Some(old) = self.sessions.get(&projection.key) {
                entry.known_turns.extend(old.known_turns.iter().cloned());
                entry.deferred_requests.clone_from(&old.deferred_requests);
                entry.external_unread_end = old.external_unread_end;
            }
            entry.external_unread = projection.unread;
            entry.external_unread_end |= projection.unread
                && projection.pending.is_empty()
                && matches!(
                    projection.status,
                    SessionStatus::Ended
                        | SessionStatus::Interrupted
                        | SessionStatus::Failed
                        | SessionStatus::Unknown
                );
            entry.sequence = if projection.key == source_key {
                event.sequence
            } else {
                None
            };
            entry.terminal = matches!(
                projection.status,
                SessionStatus::Ended | SessionStatus::Interrupted | SessionStatus::Failed
            );
            entry.terminal_status = entry.terminal.then_some(projection.status);
            entry.running_known = projection.status == SessionStatus::Running
                || (projection.status == SessionStatus::NeedsAttention
                    && projection
                        .pending
                        .iter()
                        .any(|pending| pending.lifetime == RequestLifetime::Turn));
            let deferred = entry.take_current_deferred_requests();
            self.sessions.insert(projection.key.clone(), entry);
            // 快照同样能提供回合锚点；已接收事件不再经过全局 eventId 去重。
            for request in deferred {
                self.apply_session_event(&request);
            }
        }
    }

    fn resolve_snapshot_requests(&mut self, projection: &SessionProjection) {
        let removed: Vec<_> = self.sessions[&projection.key]
            .projection
            .pending
            .iter()
            .filter(|old| {
                old.lifetime == RequestLifetime::Turn
                    && !projection
                        .pending
                        .iter()
                        .any(|new| old.id == new.id && old.turn_id == new.turn_id)
            })
            .cloned()
            .collect();
        for request in removed {
            self.resolved.insert((
                projection.key.clone(),
                request.turn_id.clone(),
                request.id.clone(),
            ));
            self.resolve_notifications(&projection.key, &request.turn_id, Some(&request.id), None);
        }
    }

    fn push_notification(&mut self, event: &EventEnvelope, kind: NotificationKind) {
        if let Some(epoch) = event
            .observation
            .as_ref()
            .and_then(|proof| proof.request_epoch(event.question_birth))
        {
            self.notification_epochs
                .insert(event.event_id.clone(), epoch.into());
        }
        self.notifications.push(NotificationRecord {
            id: event.event_id.clone(),
            key: event.session_key(),
            turn_id: event.turn_id.clone(),
            kind,
            summary: event.summary.clone(),
            project: event.project.clone(),
            occurred_at_ms: event.occurred_at_ms,
            read: false,
        });
    }

    fn resolve_notifications(
        &mut self,
        key: &SessionKey,
        turn: &Option<String>,
        request: Option<&str>,
        scope: Option<&str>,
    ) {
        for note in self.notifications.iter_mut().filter(|note| {
            note.key == *key
                && note.turn_id == *turn
                && self.notification_epochs.get(&note.id).map(String::as_str) == scope
        }) {
            if let NotificationKind::Attention {
                request_id,
                resolved,
                lifetime,
                ..
            } = &mut note.kind
                && (request.is_some() || *lifetime == RequestLifetime::Turn)
                && request.is_none_or(|id| id == request_id)
            {
                *resolved = true;
                note.read = true;
            }
        }
    }

    fn prune_history(&mut self) {
        self.notifications
            .retain(|note| self.now_ms.saturating_sub(note.occurred_at_ms) <= HISTORY_AGE_MS);
        self.notifications
            .sort_by(|a, b| a.occurred_at_ms.cmp(&b.occurred_at_ms));
        if self.notifications.len() > MAX_HISTORY {
            self.notifications
                .drain(..self.notifications.len() - MAX_HISTORY);
        }
        if !self.notification_epochs.is_empty() {
            let retained: HashSet<_> = self.notifications.iter().map(|note| &note.id).collect();
            self.notification_epochs
                .retain(|id, _| retained.contains(id));
        }
    }

    fn refresh_unread(&mut self) {
        for entry in self.sessions.values_mut() {
            entry.projection.unread = entry.external_unread
                || entry.external_unread_end
                || self
                    .notifications
                    .iter()
                    .any(|note| note.key == entry.projection.key && !note.read);
        }
    }

    pub fn mark_read(&mut self, key: &SessionKey) -> ApplyResult {
        let before = self.snapshot();
        for note in self
            .notifications
            .iter_mut()
            .filter(|note| note.key == *key)
        {
            note.read = true;
        }
        if let Some(entry) = self.sessions.get_mut(key) {
            entry.external_unread = false;
            entry.external_unread_end = false;
        }
        self.refresh_unread();
        let changed = before != self.snapshot();
        if changed {
            self.revision += 1;
        }
        ApplyResult {
            changed,
            should_notify: false,
        }
    }

    pub fn clear_history(&mut self) -> ApplyResult {
        let changed = !self.notifications.is_empty()
            || self.sessions.values().any(|entry| entry.projection.unread);
        self.notifications.clear();
        self.notification_epochs.clear();
        for entry in self.sessions.values_mut() {
            entry.external_unread = false;
            entry.external_unread_end = false;
            entry.projection.unread = false;
        }
        if changed {
            self.revision += 1;
        }
        ApplyResult {
            changed,
            should_notify: false,
        }
    }

    pub fn snapshot(&self) -> PetSnapshot {
        let sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|entry| !entry.audit_only)
            .map(|entry| entry.projection.clone())
            .collect();
        let active = self
            .sessions
            .values()
            .filter(|entry| !entry.audit_only && priority(entry, &self.notifications) > 0)
            .max_by(|a, b| {
                priority(a, &self.notifications)
                    .cmp(&priority(b, &self.notifications))
                    .then(
                        a.projection
                            .last_event_at_ms
                            .cmp(&b.projection.last_event_at_ms),
                    )
                    .then_with(|| b.projection.key.cmp(&a.projection.key))
            })
            .map(|entry| entry.projection.key.clone());
        PetSnapshot {
            revision: self.revision,
            sessions,
            notifications: self.notifications.clone(),
            active,
        }
    }

    pub fn restore(events: &[EventEnvelope], now_ms: u64) -> Self {
        let mut store = Self::default();
        for event in events {
            store.apply(event);
        }
        store.advance_time(now_ms, true);
        store
    }

    pub(crate) fn advance_time(&mut self, now_ms: u64, restarted: bool) {
        let before = self.snapshot();
        if restarted {
            for entry in self
                .sessions
                .values_mut()
                .filter(|entry| entry.running_known)
            {
                entry.running_known = false;
                restore_entry_status(entry);
            }
        }
        self.now_ms = self.now_ms.max(now_ms);
        self.prune_history();
        self.refresh_unread();
        if before != self.snapshot() {
            self.revision += 1;
        }
    }
}

// JSON 对象的字段名无法表示 SessionKey，按有序会话数组写入完整内部记录。
mod session_entries {
    use super::*;
    use serde::{Deserializer, Serializer, de::Error};

    pub(super) fn serialize<S: Serializer>(
        entries: &BTreeMap<SessionKey, SessionEntry>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(entries.values())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<SessionKey, SessionEntry>, D::Error> {
        let entries = Vec::<SessionEntry>::deserialize(deserializer)?;
        let mut sessions = BTreeMap::new();
        for entry in entries {
            if sessions
                .insert(entry.projection.key.clone(), entry)
                .is_some()
            {
                return Err(D::Error::custom("检查点包含重复会话"));
            }
        }
        Ok(sessions)
    }
}

fn status_for_outcome(outcome: TurnOutcome) -> SessionStatus {
    match outcome {
        TurnOutcome::Completed => SessionStatus::Ended,
        TurnOutcome::Interrupted => SessionStatus::Interrupted,
        TurnOutcome::Failed => SessionStatus::Failed,
        TurnOutcome::Unknown => SessionStatus::Unknown,
    }
}

fn priority(entry: &SessionEntry, notifications: &[NotificationRecord]) -> u8 {
    let projection = &entry.projection;
    if entry.confirmed_pending() {
        5
    } else if projection.status == SessionStatus::Failed {
        4
    } else if entry.external_unread_end
        || (projection.unread
            && matches!(
                projection.status,
                SessionStatus::Ended | SessionStatus::Interrupted | SessionStatus::Unknown
            ))
        || notifications.iter().any(|note| {
            note.key == projection.key
                && !note.read
                && matches!(note.kind, NotificationKind::TurnEnded { .. })
        })
    {
        3
    } else if projection.status == SessionStatus::Running {
        2
    } else {
        0
    }
}

fn restore_entry_status(entry: &mut SessionEntry) {
    entry.projection.status = if entry.confirmed_pending() {
        SessionStatus::NeedsAttention
    } else if let Some(status) = entry.terminal_status {
        status
    } else if entry.running_known {
        SessionStatus::Running
    } else {
        SessionStatus::Unknown
    };
}

fn mark_activity_unknown(entry: &mut SessionEntry) {
    entry.running_known = false;
    entry.terminal_status = None;
    entry.projection.status = if entry.projection.pending.iter().any(|pending| {
        pending.lifetime == RequestLifetime::UntilResolved && entry.pending_is_confirmed(pending)
    }) {
        SessionStatus::NeedsAttention
    } else {
        SessionStatus::Unknown
    };
}
