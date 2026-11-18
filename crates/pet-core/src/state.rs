//! 可重放的状态归约器。时间戳仅用于展示和历史保留，不作为回合因果依据。
use crate::protocol::*;
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
            known_turns: HashSet::from([event.turn_id.clone()]),
            terminal: false,
            terminal_status: None,
            has_turn_fact: false,
            deferred_requests: vec![],
            running_known: false,
            external_unread: false,
            external_unread_end: false,
        }
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
}

#[derive(Default)]
pub struct SessionStore {
    revision: u64,
    sessions: BTreeMap<SessionKey, SessionEntry>,
    notifications: Vec<NotificationRecord>,
    // 去重独立于可见历史；持久化及 24 小时以上的裁剪由存储层负责。
    seen_events: HashSet<String>,
    resolved: HashSet<(SessionKey, Option<String>, String)>,
    // 明确回合的结束事实独立于当前投影和可见历史，裁剪历史不能使它复活。
    // Task5 持久化必须保留此集合（或完整重放可恢复它的原始事件），不能只存公开快照。
    confirmed_turn_ends: HashSet<(SessionKey, String)>,
    now_ms: u64,
}

impl SessionStore {
    /// 调用者必须先在跨进程入口通过 `decode_event` 验证。
    pub fn apply(&mut self, event: &EventEnvelope) -> ApplyResult {
        if !self.seen_events.insert(event.event_id.clone()) {
            return ApplyResult::default();
        }
        let before = self.snapshot();
        self.now_ms = self
            .now_ms
            .max(event.received_at_ms.unwrap_or(event.occurred_at_ms));
        let should_notify = match &event.kind {
            EventKind::SourceDisconnected => {
                for entry in self.sessions.values_mut().filter(|entry| {
                    entry.projection.key.source == event.identity.source
                        && entry.projection.key.installation_id == event.identity.installation_id
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
            _ => self.apply_session_event(event),
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

    fn apply_session_event(&mut self, event: &EventEnvelope) -> bool {
        let key = event.session_key();
        // tombstone 即使早于请求或属于旧回合也要保留，但绝不撤销其他回合的请求。
        if let EventKind::AttentionResolved { request_id } = &event.kind {
            self.resolved
                .insert((key.clone(), event.turn_id.clone(), request_id.clone()));
        }
        let existing = self.sessions.remove(&key);
        let awaiting_causal_order = existing.as_ref().is_some_and(|entry| {
            entry.projection.status == SessionStatus::Unknown
                || (!entry.running_known && entry.terminal_status.is_none() && entry.has_turn_fact)
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
            self.resolve_notifications(&key, &event.turn_id, Some(request_id));
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
            self.confirmed_turn_ends
                .contains(&(key.clone(), turn.clone()))
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
            self.confirmed_turn_ends.insert((key.clone(), turn.clone()));
            entry.projection.pending.retain(|pending| {
                pending.lifetime == RequestLifetime::UntilResolved
                    || pending.turn_id != event.turn_id
            });
            self.resolve_notifications(&key, &event.turn_id, None);
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
            self.resolve_notifications(&key, &entry.projection.turn_id, None);
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
                entry.projection.status = if entry.projection.pending.is_empty() {
                    SessionStatus::Running
                } else {
                    SessionStatus::NeedsAttention
                };
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
                self.resolve_notifications(&key, &event.turn_id, Some(request_id));
                entry.projection.status = if !entry.projection.pending.is_empty() {
                    SessionStatus::NeedsAttention
                } else if entry.running_known {
                    SessionStatus::Running
                } else {
                    SessionStatus::Unknown
                };
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
                entry.projection.status = status_for_outcome(*outcome);
                entry
                    .projection
                    .pending
                    .retain(|pending| pending.lifetime == RequestLifetime::UntilResolved);
                self.resolve_notifications(&key, &event.turn_id, None);
                restore_entry_status(&mut entry);
                if !end_fact_notify {
                    self.push_notification(
                        event,
                        NotificationKind::TurnEnded { outcome: *outcome },
                    );
                }
                true
            }
            EventKind::SourceDisconnected | EventKind::SourceSnapshot { .. } => unreachable!(),
        };
        entry.has_turn_fact = true;
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
        let mut deferred = vec![];
        entry.deferred_requests.retain(|pending| {
            if pending.turn_id == entry.projection.turn_id {
                deferred.push(pending.clone());
                false
            } else {
                true
            }
        });
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
        if self
            .resolved
            .contains(&(key, event.turn_id.clone(), request_id.into()))
            || entry
                .projection
                .pending
                .iter()
                .any(|pending| pending.id == request_id && pending.turn_id == event.turn_id)
        {
            return false;
        }
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
            entry.projection.key.source == event.identity.source
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
                    if !entry
                        .projection
                        .pending
                        .iter()
                        .any(|new| new.id == request.id && new.turn_id == request.turn_id)
                    {
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
            self.sessions.insert(projection.key.clone(), entry);
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
            self.resolve_notifications(&projection.key, &request.turn_id, Some(&request.id));
        }
    }

    fn push_notification(&mut self, event: &EventEnvelope, kind: NotificationKind) {
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
    ) {
        for note in self
            .notifications
            .iter_mut()
            .filter(|note| note.key == *key && note.turn_id == *turn)
        {
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
            .map(|entry| entry.projection.clone())
            .collect();
        let active = self
            .sessions
            .values()
            .filter(|entry| priority(entry, &self.notifications) > 0)
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
        let before = store.snapshot();
        for entry in store.sessions.values_mut() {
            if entry.projection.status == SessionStatus::Running
                || (!entry.terminal
                    && entry.running_known
                    && entry
                        .projection
                        .pending
                        .iter()
                        .any(|pending| pending.lifetime == RequestLifetime::UntilResolved))
            {
                mark_activity_unknown(entry);
            }
        }
        store.now_ms = store.now_ms.max(now_ms);
        store.prune_history();
        store.refresh_unread();
        if before != store.snapshot() {
            store.revision += 1;
        }
        store
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
    if !projection.pending.is_empty() {
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
    entry.projection.status = if !entry.projection.pending.is_empty() {
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
    entry.projection.status = if entry
        .projection
        .pending
        .iter()
        .any(|pending| pending.lifetime == RequestLifetime::UntilResolved)
    {
        SessionStatus::NeedsAttention
    } else {
        SessionStatus::Unknown
    };
}
