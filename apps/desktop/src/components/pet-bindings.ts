import { listen } from '@tauri-apps/api/event';
import { api, observeSnapshot } from '../api';
import { builtinPets, defaultPet } from '../assets/builtin-pets';
import type { AlphaMask } from '../animation/hit-mask';
import type {
  FrameRect,
  NotificationRecord,
  PetActivityEvent,
  PetSnapshot,
  PetSoundCue,
  SessionKey,
  Settings,
} from '../types';
import type { mountPet } from './pet';
import { createNavigation } from './navigation';
import { keyId } from './notifications';
import {
  INTERACTION_ACTION_LABELS,
  PetInteractionController,
  type InteractionBubble,
  type InteractionBubbleAction,
  type InteractionOutcome,
  type InteractionStep,
  type PetClickEvent,
} from './pet-interactions';
import { isSubagentSession, presentation, withBubbles } from './reminder';
import { DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS } from './settings';

type BubbleAction = 'session' | 'application' | 'copy' | 'close' | 'close_all' | InteractionBubbleAction;
type BubbleBackdrop = FrameRect & { radius: number };
type BubbleInput = {
  stamp: number;
  buttons: { bubble: number; action: BubbleAction; rect: FrameRect }[];
  backdrops?: BubbleBackdrop[];
};
type Pose = { mask: AlphaMask; frame: FrameRect };
export type InputPose = Pose & { bubbleInput?: BubbleInput };

type BubbleKind = 'reminder' | 'interaction' | 'activity' | 'error';

interface EphemeralBubble {
  id: string;
  kind: 'interaction' | 'activity';
  title: string;
  detail: string;
  actions: InteractionBubbleAction[];
  createdAtMs: number;
  expiresAtMs: number;
  order: number;
  timer?: ReturnType<typeof setTimeout>;
}

interface RenderBubble {
  id: string;
  kind: BubbleKind;
  title: string;
  detail: string;
  order: number;
  rank: number;
  key?: SessionKey;
  actions?: InteractionBubbleAction[];
}

interface BubbleView {
  id: string;
  element: HTMLElement;
  title: HTMLElement;
  detail: HTMLElement;
  close: HTMLButtonElement;
  controls: HTMLDivElement;
  controlsKey?: string;
}

const PET_LOAD_ERROR_ID = 'pet-load-error';
const DISMISSED_STORAGE_KEY = 'codex-pet:dismissed-reminders:v1';
const MAX_DISMISSED = 200;
const DEFAULT_BUBBLE_RADIUS = 14;
const MAX_VISIBLE_BUBBLES = 4;
const MAX_EPHEMERAL_BUBBLES = 3;
const INTERACTION_REPLACE_WINDOW_MS = 450;
const OVERFLOW_EPSILON_PX = 0.5;

export function alertCueForNotification(record: NotificationRecord): PetSoundCue {
  if (record.kind.type === 'attention') {
    return record.kind.reason === 'approval' ? 'alert_approval' : 'alert_answer';
  }
  if (record.kind.outcome === 'failed') return 'alert_failed';
  if (record.kind.outcome === 'interrupted') return 'alert_interrupted';
  return 'alert_complete';
}

function notificationSoundPriority(record: NotificationRecord): number {
  if (record.kind.type === 'attention') return 5;
  if (record.kind.outcome === 'failed') return 4;
  if (record.kind.outcome === 'interrupted') return 3;
  return 2;
}

export function ambientCueForActivity(event: PetActivityEvent): PetSoundCue | undefined {
  if (event.phase === 'end') return undefined;
  if (event.state === 'jumping') return 'ambient_hop';
  if (event.state === 'waving') return 'ambient_wave';
  if (event.thought || event.effect) return 'ambient_murmur';
  return undefined;
}

function loadDismissed(): Set<string> {
  try {
    if (typeof localStorage === 'undefined' || typeof localStorage.getItem !== 'function') {
      return new Set();
    }
    const raw = localStorage.getItem(DISMISSED_STORAGE_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    const ids = parsed.filter((item): item is string => typeof item === 'string');
    return new Set(ids.slice(-MAX_DISMISSED));
  } catch {
    return new Set();
  }
}

function saveDismissed(dismissed: Set<string>): void {
  try {
    if (typeof localStorage === 'undefined' || typeof localStorage.setItem !== 'function') {
      return;
    }
    localStorage.setItem(DISMISSED_STORAGE_KEY, JSON.stringify(Array.from(dismissed)));
  } catch {
    // 忽略受限环境中的本地存储错误
  }
}

export function bindReminder(
  canvas: HTMLCanvasElement,
  pet: ReturnType<typeof mountPet>,
  sendPose: (pose: InputPose) => void,
  setFps: (fps: 30 | 60) => void,
  validationState?: string,
  validationFps?: number,
) {
  const stackEl = document.createElement('div');
  stackEl.className = 'bubble-stack';
  stackEl.hidden = true;
  document.body.append(stackEl);
  const closeAllButton = document.createElement('button');
  closeAllButton.type = 'button';
  closeAllButton.className = 'bubble-close-all';
  closeAllButton.dataset.action = 'close_all';
  closeAllButton.setAttribute('aria-label', '关闭全部气泡');
  closeAllButton.title = '关闭全部气泡';
  closeAllButton.hidden = true;
  closeAllButton.onclick = () => dismissAllBubbles();
  const observer = new ResizeObserver(() => handleBubbleLayoutChange());
  observer.observe(canvas);
  observer.observe(stackEl);
  observer.observe(closeAllButton);

  let base: Pose | undefined,
    rects: FrameRect[] = [],
    measuredSize: { width: number; height: number } | undefined,
    merged = new WeakMap<AlphaMask, AlphaMask>();
  let input: BubbleInput | undefined;
  let stamp = 0;
  let targetChanged = false;
  let bubbleOrder = 0;
  let renderedIds: string[] = [];
  let renderedTotalCount = 0;
  let lastEntriesSignature = '';
  let lastFitCanvasHeight = 0;
  const views = new Map<string, BubbleView>();
  const reminderOrder = new Map<string, number>();
  const reminderAutoCloseTimers = new Map<
    string,
    { timer: ReturnType<typeof setTimeout>; durationMs: number }
  >();
  let petLoadErrorOrder: number | undefined;
  const ephemeralBubbles: EphemeralBubble[] = [];

  const dismissed = loadDismissed();
  const completedAnimations = new Set<string>();
  let activeAnimation: { state: string; key: string; fallbackState: string } | undefined;
  let snapshot: PetSnapshot | undefined,
    settings: Settings | undefined,
    selectedPet = defaultPet.manifest.id,
    petName = defaultPet.manifest.name;
  let change = 0,
    stopped = false;
  const stops: (() => void)[] = [];
  const interactions = new PetInteractionController();
  let interactionQueue: InteractionStep[] = [];
  let interactionStep: { state: string; timed: boolean } | undefined;
  let interactionStepTimer: ReturnType<typeof setTimeout> | undefined;
  let autonomousActivity: { id: string; state?: string } | undefined;
  let soundedActivityId: string | undefined;
  let lastTaskKey: string | undefined;

  function clearAutonomousActivity() {
    autonomousActivity = undefined;
    soundedActivityId = undefined;
  }

  function clearEphemeralTimer(entry: EphemeralBubble) {
    if (entry.timer !== undefined) {
      clearTimeout(entry.timer);
      entry.timer = undefined;
    }
  }

  function clearAllEphemeralBubbles() {
    for (const entry of ephemeralBubbles) clearEphemeralTimer(entry);
    ephemeralBubbles.length = 0;
  }

  function resolveReminderAutoCloseMs(): number {
    const seconds = settings?.bubbleAutoCloseSeconds;
    if (typeof seconds === 'number' && Number.isFinite(seconds)) {
      return seconds <= 0 ? 0 : Math.round(seconds * 1000);
    }
    return DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS * 1000;
  }

  function clearReminderAutoCloseTimer(id: string) {
    const entry = reminderAutoCloseTimers.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    reminderAutoCloseTimers.delete(id);
  }

  function clearAllReminderAutoCloseTimers() {
    for (const entry of reminderAutoCloseTimers.values()) {
      clearTimeout(entry.timer);
    }
    reminderAutoCloseTimers.clear();
  }

  function autoDismissReminder(id: string) {
    clearReminderAutoCloseTimer(id);
    if (stopped || dismissed.has(id)) return;
    rememberDismissed([id]);
    reminderOrder.delete(id);
    renderStack(true);
  }

  function scheduleReminderAutoClose(id: string, durationMs: number) {
    clearReminderAutoCloseTimer(id);
    if (stopped || durationMs <= 0) return;
    const timer = setTimeout(() => {
      reminderAutoCloseTimers.delete(id);
      autoDismissReminder(id);
    }, durationMs);
    reminderAutoCloseTimers.set(id, { timer, durationMs });
  }

  function refreshReminderAutoClose(id: string) {
    if (stopped) return;
    const autoCloseMs = resolveReminderAutoCloseMs();
    if (autoCloseMs <= 0) return;
    if (!renderedIds.includes(id)) return;
    if (views.get(id)?.element.dataset.bubbleKind !== 'reminder') return;
    scheduleReminderAutoClose(id, autoCloseMs);
  }

  function syncReminderAutoCloseTimers(visible: readonly RenderBubble[]) {
    const autoCloseMs = resolveReminderAutoCloseMs();
    if (stopped || autoCloseMs <= 0) {
      clearAllReminderAutoCloseTimers();
      return;
    }
    const visibleReminderIds = new Set<string>();
    for (const entry of visible) {
      if (entry.kind === 'reminder') visibleReminderIds.add(entry.id);
    }
    for (const id of Array.from(reminderAutoCloseTimers.keys())) {
      if (!visibleReminderIds.has(id)) {
        clearReminderAutoCloseTimer(id);
      }
    }
    for (const entry of visible) {
      if (entry.kind !== 'reminder') continue;
      const existing = reminderAutoCloseTimers.get(entry.id);
      if (existing && existing.durationMs === autoCloseMs) continue;
      scheduleReminderAutoClose(entry.id, autoCloseMs);
    }
  }

  function removeEphemeralBubble(id: string): boolean {
    const index = ephemeralBubbles.findIndex((entry) => entry.id === id);
    if (index === -1) return false;
    clearEphemeralTimer(ephemeralBubbles[index]);
    ephemeralBubbles.splice(index, 1);
    return true;
  }

  function clearInteractionAnimation() {
    if (interactionStepTimer !== undefined) {
      clearTimeout(interactionStepTimer);
      interactionStepTimer = undefined;
    }
    interactionQueue = [];
    interactionStep = undefined;
  }

  function notifySubagentsEnabled(): boolean {
    return settings?.notifySubagents ?? true;
  }

  function hasUrgentAttention(): boolean {
    const allowSubagents = notifySubagentsEnabled();
    return Boolean(
      snapshot?.sessions.some(
        (session) =>
          session.status === 'needs_attention' && (allowSubagents || !isSubagentSession(session)),
      ),
    );
  }

  function scheduleEphemeralExpiry(entry: EphemeralBubble, delayMs: number) {
    clearEphemeralTimer(entry);
    entry.timer = setTimeout(() => {
      entry.timer = undefined;
      if (stopped) return;
      if (removeEphemeralBubble(entry.id)) {
        renderStack(true);
      }
    }, Math.max(0, delayMs));
  }

  function trimEphemeralBubbles() {
    while (ephemeralBubbles.length > MAX_EPHEMERAL_BUBBLES) {
      const dropIndex = ephemeralBubbles.findIndex((entry) => entry.actions.length === 0);
      const index = dropIndex >= 0 ? dropIndex : 0;
      clearEphemeralTimer(ephemeralBubbles[index]);
      ephemeralBubbles.splice(index, 1);
    }
  }

  function pushInteractionBubble(
    bubble: InteractionBubble,
    nowMs: number,
    options?: { replacePanel?: boolean; replaceRecentInteraction?: boolean },
  ) {
    const delayMs = Math.max(0, bubble.expiresAtMs - nowMs);
    let existingIndex = -1;
    if (options?.replacePanel || bubble.actions.length > 0) {
      existingIndex = ephemeralBubbles.findIndex(
        (entry) => entry.kind === 'interaction' && entry.actions.length > 0,
      );
    }
    if (existingIndex === -1 && options?.replaceRecentInteraction) {
      for (let i = ephemeralBubbles.length - 1; i >= 0; i--) {
        const candidate = ephemeralBubbles[i];
        if (
          candidate.kind === 'interaction' &&
          (bubble.actions.length > 0 || nowMs - candidate.createdAtMs <= INTERACTION_REPLACE_WINDOW_MS)
        ) {
          existingIndex = i;
          break;
        }
      }
    }
    if (existingIndex >= 0) {
      const previous = ephemeralBubbles[existingIndex];
      clearEphemeralTimer(previous);
      const updated: EphemeralBubble = {
        id: bubble.id,
        kind: 'interaction',
        title: bubble.title,
        detail: bubble.detail,
        actions: bubble.actions,
        createdAtMs: nowMs,
        expiresAtMs: bubble.expiresAtMs,
        order: previous.order,
      };
      ephemeralBubbles[existingIndex] = updated;
      scheduleEphemeralExpiry(updated, delayMs);
    } else {
      const created: EphemeralBubble = {
        id: bubble.id,
        kind: 'interaction',
        title: bubble.title,
        detail: bubble.detail,
        actions: bubble.actions,
        createdAtMs: nowMs,
        expiresAtMs: bubble.expiresAtMs,
        order: ++bubbleOrder,
      };
      ephemeralBubbles.push(created);
      scheduleEphemeralExpiry(created, delayMs);
      trimEphemeralBubbles();
    }
    renderStack(true);
  }

  function pushActivityBubble(event: PetActivityEvent, nowMs: number) {
    if (!event.thought) return;
    const bubbleId = `pet-activity:${event.id}`;
    const durationMs = Math.max(1200, Math.min(8000, event.thoughtDurationMs ?? 3600));
    const existingIndex = ephemeralBubbles.findIndex((entry) => entry.id === bubbleId);
    if (existingIndex >= 0) {
      const previous = ephemeralBubbles[existingIndex];
      clearEphemeralTimer(previous);
      const updated: EphemeralBubble = {
        ...previous,
        title: petName,
        detail: event.thought,
        createdAtMs: nowMs,
        expiresAtMs: nowMs + durationMs,
      };
      ephemeralBubbles[existingIndex] = updated;
      scheduleEphemeralExpiry(updated, durationMs);
    } else {
      const created: EphemeralBubble = {
        id: bubbleId,
        kind: 'activity',
        title: petName,
        detail: event.thought,
        actions: [],
        createdAtMs: nowMs,
        expiresAtMs: nowMs + durationMs,
        order: ++bubbleOrder,
      };
      ephemeralBubbles.push(created);
      scheduleEphemeralExpiry(created, durationMs);
      trimEphemeralBubbles();
    }
    renderStack(true);
  }

  function restoreAfterInteraction(timestamp: number) {
    clearInteractionAnimation();
    if (snapshot) {
      sync();
    } else if (!validationState) {
      pet.renderer.setState('idle', timestamp);
    }
  }

  function playNextInteractionStep(nowMs: number) {
    if (stopped || validationState) {
      clearInteractionAnimation();
      return;
    }
    if (interactionStepTimer !== undefined) {
      clearTimeout(interactionStepTimer);
      interactionStepTimer = undefined;
    }
    while (interactionQueue.length > 0) {
      const next = interactionQueue.shift()!;
      const applied = pet.renderer.setState(next.state, nowMs, true);
      if (applied === false) continue;
      const timed = next.durationMs !== undefined;
      interactionStep = { state: next.state, timed };
      const timeoutMs = timed ? Math.max(100, next.durationMs!) : 4000;
      interactionStepTimer = setTimeout(() => {
        interactionStepTimer = undefined;
        if (stopped) return;
        playNextInteractionStep(performance.now());
      }, timeoutMs);
      return;
    }
    restoreAfterInteraction(nowMs);
  }

  function applyInteraction(
    outcome: InteractionOutcome | undefined,
    nowMs: number,
    options?: { fromAction?: boolean; fromClick?: boolean; sourceBubbleId?: string },
  ) {
    if (!outcome || stopped) return;
    if (!validationState && outcome.sound && (settings?.interactionSound ?? true)) {
      void Promise.resolve(api.playPetSound(outcome.sound)).catch(console.error);
    }
    if (outcome.effect) {
      pet.renderer.triggerInteractionEffect?.(outcome.effect, nowMs);
    }
    if (outcome.openNotifications) {
      void api.openNotifications().catch(console.error);
    }
    if (outcome.closeBubble) {
      let removed = false;
      if (options?.sourceBubbleId) {
        removed = removeEphemeralBubble(options.sourceBubbleId);
      } else {
        const panel = ephemeralBubbles.find((entry) => entry.kind === 'interaction' && entry.actions.length > 0);
        if (panel) removed = removeEphemeralBubble(panel.id);
      }
      if (removed) renderStack(true);
    }
    if (!validationState && outcome.steps && outcome.steps.length > 0) {
      activeAnimation = undefined;
      interactionQueue = [...outcome.steps];
      playNextInteractionStep(nowMs);
    }
    if (outcome.bubble) {
      pushInteractionBubble(outcome.bubble, nowMs, {
        replacePanel: options?.fromAction,
        replaceRecentInteraction: options?.fromClick,
      });
    }
  }

  function handlePetClick(event: PetClickEvent) {
    if (stopped) return;
    if (autonomousActivity) {
      clearAutonomousActivity();
      void api.interruptPetActivity().catch(() => {});
    }
    const now = performance.now();
    applyInteraction(
      interactions.handleClick(event, now, {
        petName,
        snapshot,
        notifySubagents: notifySubagentsEnabled(),
      }),
      now,
      { fromClick: true },
    );
  }

  function handleDragEnd() {
    clearAutonomousActivity();
    if (stopped || validationState || hasUrgentAttention()) return;
    const now = performance.now();
    applyInteraction(
      interactions.handleDragEnd(now, {
        petName,
        snapshot,
        notifySubagents: notifySubagentsEnabled(),
      }),
      now,
    );
  }

  function handleAutonomousActivity(event: PetActivityEvent) {
    if (stopped || validationState) return;
    const now = performance.now();
    if (event.phase === 'end') {
      if (!autonomousActivity || autonomousActivity.id === event.id) {
        clearAutonomousActivity();
        if (!interactionStep) {
          if (snapshot) sync();
          else pet.renderer.setState('idle', now);
        }
      }
      return;
    }
    if (interactionStep || hasUrgentAttention() || activeAnimation) return;
    autonomousActivity = {
      id: event.id,
      state: event.state ?? autonomousActivity?.state,
    };
    if (event.state) {
      activeAnimation = undefined;
      pet.renderer.setState(event.state, now, true);
    }
    if (event.effect) {
      pet.renderer.triggerInteractionEffect?.(event.effect, now);
    }
    if (event.thought) {
      pushActivityBubble(event, now);
    }
    const ambientCue = ambientCueForActivity(event);
    if (ambientCue && soundedActivityId !== event.id && (settings?.ambientSound ?? true)) {
      soundedActivityId = event.id;
      void Promise.resolve(api.playPetSound(ambientCue)).catch(console.error);
    }
  }

  function rememberDismissed(ids: Iterable<string>) {
    let changed = false;
    for (const id of ids) {
      if (dismissed.has(id)) continue;
      dismissed.add(id);
      changed = true;
    }
    while (dismissed.size > MAX_DISMISSED) {
      const oldest = dismissed.values().next().value;
      if (oldest === undefined) break;
      dismissed.delete(oldest);
      changed = true;
    }
    if (changed) saveDismissed(dismissed);
  }

  function pruneImportedDismissed(value: PetSnapshot) {
    if (dismissed.size === 0) return;
    const importedEnds = new Set(value.importedUnreadEnds?.map(keyId));
    let changed = false;
    for (const id of Array.from(dismissed)) {
      if (!id.startsWith('["imported",')) continue;
      try {
        const parsed = JSON.parse(id) as unknown;
        if (Array.isArray(parsed) && parsed[0] === 'imported' && typeof parsed[1] === 'string') {
          if (!importedEnds.has(parsed[1])) {
            dismissed.delete(id);
            changed = true;
          }
        }
      } catch {
        // 忽略非预期格式
      }
    }
    if (changed) saveDismissed(dismissed);
  }

  function collectBubbleEntries(): RenderBubble[] {
    const entries: RenderBubble[] = [];
    const activeReminderIds = new Set<string>();
    if (snapshot) {
      const result = presentation(snapshot, settings?.mutedSources, notifySubagentsEnabled());
      const undismissed = result.reminders.filter((item) => !dismissed.has(item.reminderId));
      const fresh = undismissed
        .filter((item) => !reminderOrder.has(item.reminderId))
        .sort((a, b) => a.lastEventAtMs - b.lastEventAtMs || a.rank - b.rank);
      for (const item of fresh) {
        reminderOrder.set(item.reminderId, ++bubbleOrder);
      }
      for (const item of undismissed) {
        activeReminderIds.add(item.reminderId);
        entries.push({
          id: item.reminderId,
          kind: 'reminder',
          title: item.title,
          detail: item.detail,
          order: reminderOrder.get(item.reminderId)!,
          rank: item.rank,
          key: item.key,
        });
      }
    }
    for (const knownId of Array.from(reminderOrder.keys())) {
      if (!activeReminderIds.has(knownId)) reminderOrder.delete(knownId);
    }
    if (petLoadErrorOrder !== undefined) {
      entries.push({
        id: PET_LOAD_ERROR_ID,
        kind: 'error',
        title: '无法加载所选桌宠',
        detail: '请在设置中重新选择宠物',
        order: petLoadErrorOrder,
        rank: 6,
      });
    }
    for (const item of ephemeralBubbles) {
      entries.push({
        id: item.id,
        kind: item.kind,
        title: item.title,
        detail: item.detail,
        order: item.order,
        rank: item.actions.length > 0 ? 2 : 1,
        actions: item.actions,
      });
    }
    return entries;
  }

  function selectVisibleBubbles(entries: readonly RenderBubble[], maxVisible: number): RenderBubble[] {
    const limit = Math.max(0, Math.min(maxVisible, entries.length));
    if (limit === 0) return [];
    if (entries.length <= limit) {
      return [...entries].sort((a, b) => a.order - b.order);
    }
    // 超过同屏上限或可用高度时，优先保留错误与待审批/待回答提醒，其余保留最新叠加的气泡
    const pinned = entries.filter((entry) => entry.rank >= 5).sort((a, b) => b.order - a.order);
    const others = entries.filter((entry) => entry.rank < 5).sort((a, b) => b.order - a.order);
    const kept = [...pinned.slice(0, limit)];
    const remaining = limit - kept.length;
    if (remaining > 0) kept.push(...others.slice(0, remaining));
    kept.sort((a, b) => a.order - b.order);
    return kept;
  }

  function doesRenderedStackOverflow(visibleElements: readonly HTMLElement[]): boolean {
    if (visibleElements.length <= 1 || stackEl.hidden) return false;
    const canvasBox = canvas.getBoundingClientRect();
    if (!canvasBox || canvasBox.width <= 0 || canvasBox.height <= 0) return false;

    let minTop = Number.POSITIVE_INFINITY;
    let maxBottom = Number.NEGATIVE_INFINITY;
    let maxWidth = 0;
    for (const element of visibleElements) {
      const box = element.getBoundingClientRect();
      if (!box || box.width <= 0 || box.height <= 0) return false;
      minTop = Math.min(minTop, box.top);
      maxBottom = Math.max(maxBottom, box.top + box.height);
      maxWidth = Math.max(maxWidth, box.width);
    }

    // 任意气泡顶部超出窗口（canvas）顶部边界，头部必然被截断
    if (minTop < canvasBox.top - OVERFLOW_EPSILON_PX) {
      return true;
    }

    // 容器具备有效布局盒时，检查气泡是否超出 .bubble-stack 可用上下边界
    const stackBox = stackEl.getBoundingClientRect();
    if (
      stackBox &&
      stackBox.width >= maxWidth - 1 &&
      stackBox.height > 0 &&
      stackBox.top >= canvasBox.top - 1 &&
      stackBox.top + stackBox.height <= canvasBox.top + canvasBox.height + 1
    ) {
      if (minTop < stackBox.top - OVERFLOW_EPSILON_PX) {
        return true;
      }
      if (maxBottom > stackBox.top + stackBox.height + OVERFLOW_EPSILON_PX) {
        return true;
      }
    }

    return false;
  }

  function createBubbleView(id: string): BubbleView {
    const element = document.createElement('aside');
    element.className = 'bubble';
    element.dataset.bubbleId = id;
    const header = document.createElement('div');
    header.className = 'bubble-header';
    const title = document.createElement('strong');
    const close = document.createElement('button');
    close.type = 'button';
    close.className = 'bubble-close';
    close.dataset.action = 'close';
    close.setAttribute('aria-label', '关闭提示框');
    close.title = '关闭';
    close.textContent = '×';
    header.append(title, close);
    const detail = document.createElement('span');
    detail.className = 'bubble-detail';
    const controls = document.createElement('div');
    controls.className = 'bubble-controls';
    element.append(header, detail, controls);
    observer.observe(element);
    return { id, element, title, detail, close, controls };
  }

  function dismissBubble(id: string, kind: BubbleKind, key?: SessionKey) {
    if (kind === 'error') {
      petLoadErrorOrder = undefined;
      renderStack(true);
      return;
    }
    if (kind === 'interaction' || kind === 'activity') {
      if (removeEphemeralBubble(id)) {
        renderStack(true);
      }
      return;
    }
    clearReminderAutoCloseTimer(id);
    rememberDismissed([id]);
    reminderOrder.delete(id);
    renderStack(true);
    if (key) {
      void Promise.resolve(api.markRead(key)).catch(console.error);
    }
  }

  function dismissAllBubbles() {
    const reminderIds: string[] = [];
    const keysToMarkRead: SessionKey[] = [];
    const seenKeys = new Set<string>();
    if (snapshot) {
      const result = presentation(snapshot, settings?.mutedSources, notifySubagentsEnabled());
      for (const item of result.reminders) {
        if (!dismissed.has(item.reminderId)) {
          reminderIds.push(item.reminderId);
          const sid = keyId(item.key);
          if (!seenKeys.has(sid)) {
            seenKeys.add(sid);
            keysToMarkRead.push(item.key);
          }
        }
        reminderOrder.delete(item.reminderId);
      }
    }
    if (reminderIds.length > 0) {
      rememberDismissed(reminderIds);
    }
    petLoadErrorOrder = undefined;
    clearAllReminderAutoCloseTimers();
    clearAllEphemeralBubbles();
    renderStack(true);
    if (keysToMarkRead.length === 1) {
      void Promise.resolve(api.markRead(keysToMarkRead[0])).catch(console.error);
    } else if (keysToMarkRead.length > 1) {
      void Promise.resolve(api.markReadMany(keysToMarkRead)).catch(console.error);
    }
  }

  function mountVisibleBubbles(visible: readonly RenderBubble[], totalCount: number) {
    const visibleSet = new Set(visible.map((entry) => entry.id));
    const showCloseAll = totalCount > 1;

    for (const pair of Array.from(views.entries())) {
      const id = pair[0];
      const view = pair[1];
      if (!visibleSet.has(id)) {
        observer.unobserve?.(view.element);
        view.element.remove();
        views.delete(id);
      }
    }

    for (let index = 0; index < visible.length; index++) {
      const entry = visible[index];
      let view = views.get(entry.id);
      if (!view) {
        view = createBubbleView(entry.id);
        views.set(entry.id, view);
      }
      view.element.hidden = false;
      view.element.dataset.bubbleIndex = String(index);
      view.element.dataset.bubbleKind = entry.kind;
      view.title.textContent = entry.title;
      view.detail.textContent = entry.detail;
      view.close.hidden = false;
      view.close.textContent = '×';
      view.close.onclick = () => dismissBubble(entry.id, entry.kind, entry.key);

      if (entry.kind === 'reminder' && entry.key) {
        const controlsKey = `nav:${keyId(entry.key)}`;
        view.element.classList.toggle('has-actions', true);
        if (view.controlsKey !== controlsKey) {
          view.controlsKey = controlsKey;
          view.controls.replaceChildren(
            createNavigation(
              document,
              entry.key,
              {
                open: (key, intent) => {
                  refreshReminderAutoClose(entry.id);
                  return api.openSession(key, intent);
                },
                copy: (key) => {
                  refreshReminderAutoClose(entry.id);
                  return api.copySession(key);
                },
              },
              true,
              () => {
                refreshReminderAutoClose(entry.id);
                handleBubbleLayoutChange();
              },
            ),
          );
        }
      } else if (entry.actions && entry.actions.length > 0) {
        const controlsKey = `interaction:${entry.actions.join(',')}`;
        view.element.classList.toggle('has-actions', true);
        if (view.controlsKey !== controlsKey) {
          view.controlsKey = controlsKey;
          const bar = document.createElement('div');
          bar.className = 'actions';
          for (const action of entry.actions) {
            const button = document.createElement('button');
            button.type = 'button';
            button.dataset.action = action;
            button.textContent = INTERACTION_ACTION_LABELS[action];
            button.onclick = () => {
              if (stopped) return;
              const now = performance.now();
              applyInteraction(
                interactions.handleAction(action, now, {
                  petName,
                  snapshot,
                  notifySubagents: notifySubagentsEnabled(),
                }),
                now,
                {
                  fromAction: true,
                  sourceBubbleId: entry.id,
                },
              );
            };
            bar.append(button);
          }
          view.controls.replaceChildren(bar);
        }
      } else {
        view.element.classList.toggle('has-actions', false);
        if (view.controlsKey !== undefined) {
          view.controlsKey = undefined;
          view.controls.replaceChildren();
        }
      }
    }

    closeAllButton.hidden = !showCloseAll;
    if (showCloseAll) {
      closeAllButton.textContent = `全部关闭 (${totalCount})`;
    }
    const desiredNodes: HTMLElement[] = showCloseAll
      ? [closeAllButton, ...visible.map((entry) => views.get(entry.id)!.element)]
      : visible.map((entry) => views.get(entry.id)!.element);
    const orderMatches =
      stackEl.children.length === desiredNodes.length &&
      desiredNodes.every((node, index) => stackEl.children[index] === node);
    if (!orderMatches) {
      stackEl.replaceChildren(...desiredNodes);
    }

    stackEl.hidden = visible.length === 0;
    stackEl.classList.toggle('is-stacked', visible.length > 1);
    stackEl.classList.toggle('has-close-all', showCloseAll);
  }

  function renderStack(forceInvalidate = false, forceQueue = false) {
    const entries = collectBubbleEntries();
    const totalCount = entries.length;
    const entriesSignature = JSON.stringify(
      entries.map((entry) => [entry.id, entry.title, entry.detail, entry.rank, entry.actions ?? null]),
    );
    const canvasHeight = canvas.getBoundingClientRect()?.height ?? 0;
    const previousControlsKeys = new Map<string, string | undefined>();
    views.forEach((view, id) => {
      previousControlsKeys.set(id, view.controlsKey);
    });

    const maxAllowed = Math.min(totalCount, MAX_VISIBLE_BUBBLES);
    let limit = maxAllowed;
    if (
      totalCount > 0 &&
      !forceInvalidate &&
      entriesSignature === lastEntriesSignature &&
      canvasHeight <= lastFitCanvasHeight + OVERFLOW_EPSILON_PX &&
      renderedIds.length > 0
    ) {
      limit = Math.max(1, Math.min(renderedIds.length, maxAllowed));
    }

    let visible = selectVisibleBubbles(entries, limit);
    mountVisibleBubbles(visible, totalCount);
    while (
      visible.length > 1 &&
      doesRenderedStackOverflow(visible.map((entry) => views.get(entry.id)!.element))
    ) {
      limit = visible.length - 1;
      visible = selectVisibleBubbles(entries, limit);
      mountVisibleBubbles(visible, totalCount);
    }

    const visibleIds = visible.map((entry) => entry.id);
    const invalidated =
      forceInvalidate ||
      totalCount !== renderedTotalCount ||
      visibleIds.length !== renderedIds.length ||
      visibleIds.some((id, index) => renderedIds[index] !== id) ||
      visible.some((entry) => views.get(entry.id)?.controlsKey !== previousControlsKeys.get(entry.id));

    renderedIds = visibleIds;
    renderedTotalCount = totalCount;
    lastEntriesSignature = entriesSignature;
    if (canvasHeight > 0) {
      lastFitCanvasHeight = canvasHeight;
    }
    syncReminderAutoCloseTimers(visible);
    if (invalidated) {
      ++stamp;
      targetChanged = true;
    }
    measure(forceQueue);
  }

  function handleBubbleLayoutChange(forceQueue = false) {
    if (stopped) return;
    if (!stackEl.hidden && renderedIds.length > 0) {
      const canvasHeight = canvas.getBoundingClientRect()?.height ?? 0;
      const canExpand =
        canvasHeight > lastFitCanvasHeight + OVERFLOW_EPSILON_PX &&
        renderedIds.length < Math.min(renderedTotalCount, MAX_VISIBLE_BUBBLES);
      const currentElements = renderedIds
        .map((id) => views.get(id)?.element)
        .filter((el): el is HTMLElement => el !== undefined);
      if (canExpand || doesRenderedStackOverflow(currentElements)) {
        renderStack(false, forceQueue);
        return;
      }
      if (canvasHeight > 0) {
        lastFitCanvasHeight = canvasHeight;
      }
    }
    measure(forceQueue);
  }

  function queue() {
    if (!base) return;
    const mask = base.mask,
      frame = base.frame;
    if (rects.length === 0) {
      sendPose(base);
      return;
    }
    const shape = merged.get(mask) ?? withBubbles(mask, rects);
    merged.set(mask, shape);
    sendPose({ mask: shape, frame, bubbleInput: input });
  }

  function measure(forceQueue = false) {
    const c = canvas.getBoundingClientRect();
    if (!c) {
      if (forceQueue) queue();
      return;
    }
    const targetWidth = base?.mask.width ?? canvas.width;
    const targetHeight = base?.mask.height ?? canvas.height;
    const sx = c.width ? targetWidth / c.width : 0,
      sy = c.height ? targetHeight / c.height : 0;
    const bounds = (b: DOMRect): FrameRect => ({
      x: (b.left - c.left) * sx,
      y: (b.top - c.top) * sy,
      width: b.width * sx,
      height: b.height * sy,
    });
    const clampRect = (b: DOMRect): FrameRect => {
      const raw = bounds(b);
      const x1 = Math.max(0, Math.min(targetWidth, raw.x));
      const y1 = Math.max(0, Math.min(targetHeight, raw.y));
      const x2 = Math.max(0, Math.min(targetWidth, raw.x + raw.width));
      const y2 = Math.max(0, Math.min(targetHeight, raw.y + raw.height));
      return { x: x1, y: y1, width: x2 - x1, height: y2 - y1 };
    };

    const showing = !stackEl.hidden && !!c.width && !!c.height && targetWidth > 0 && targetHeight > 0;
    const scale = Math.min(sx, sy);
    const nextRects: FrameRect[] = [];
    const backdrops: BubbleBackdrop[] = [];
    const buttons: { bubble: number; action: BubbleAction; rect: FrameRect }[] = [];

    if (showing) {
      const bubbleElements = Array.from(stackEl.querySelectorAll<HTMLElement>('aside.bubble:not([hidden])'));
      for (let index = 0; index < bubbleElements.length; index++) {
        const bubble = bubbleElements[index];
        const bubbleIndex = Number.parseInt(bubble.dataset.bubbleIndex ?? String(index), 10);
        const bubbleBox = bubble.getBoundingClientRect();
        if (bubbleBox.width > 0 && bubbleBox.height > 0) {
          nextRects.push(bounds(bubbleBox));
          const backdropRect = clampRect(bubbleBox);
          if (backdropRect.width > 0 && backdropRect.height > 0) {
            const cssRadius =
              typeof getComputedStyle === 'function'
                ? Number.parseFloat(getComputedStyle(bubble).borderTopLeftRadius)
                : Number.NaN;
            backdrops.push({
              ...backdropRect,
              radius: Math.min(
                Math.max(0, (Number.isFinite(cssRadius) ? cssRadius : DEFAULT_BUBBLE_RADIUS) * scale),
                Math.min(backdropRect.width, backdropRect.height) / 2,
              ),
            });
          }
        }
        for (const button of Array.from(
          bubble.querySelectorAll<HTMLButtonElement>('button[data-action]:not([hidden])'),
        )) {
          const buttonRect = clampRect(button.getBoundingClientRect());
          if (buttonRect.width > 0 && buttonRect.height > 0) {
            buttons.push({
              bubble: Number.isFinite(bubbleIndex) ? bubbleIndex : index,
              action: button.dataset.action as BubbleAction,
              rect: buttonRect,
            });
          }
        }
      }
      if (!closeAllButton.hidden && closeAllButton.parentNode === stackEl) {
        const closeAllBox = closeAllButton.getBoundingClientRect();
        if (closeAllBox.width > 0 && closeAllBox.height > 0) {
          nextRects.push(bounds(closeAllBox));
          const backdropRect = clampRect(closeAllBox);
          if (backdropRect.width > 0 && backdropRect.height > 0) {
            const cssRadius =
              typeof getComputedStyle === 'function'
                ? Number.parseFloat(getComputedStyle(closeAllButton).borderTopLeftRadius)
                : Number.NaN;
            const pillRadius = Math.min(closeAllBox.width, closeAllBox.height) / 2;
            backdrops.push({
              ...backdropRect,
              radius: Math.min(
                Math.max(0, (Number.isFinite(cssRadius) ? cssRadius : pillRadius) * scale),
                Math.min(backdropRect.width, backdropRect.height) / 2,
              ),
            });
            buttons.push({
              bubble: 0,
              action: 'close_all',
              rect: backdropRect,
            });
          }
        }
      }
    }

    const sizeChanged =
      base !== undefined &&
      (!measuredSize || measuredSize.width !== targetWidth || measuredSize.height !== targetHeight);
    const shapeChanged = JSON.stringify(nextRects) !== JSON.stringify(rects);
    const backdropsChanged = JSON.stringify(backdrops) !== JSON.stringify(input?.backdrops ?? []);
    const buttonsChanged = JSON.stringify(buttons) !== JSON.stringify(input?.buttons ?? []);
    if (base) measuredSize = { width: targetWidth, height: targetHeight };
    if (!shapeChanged && !backdropsChanged && !buttonsChanged && !targetChanged && !sizeChanged) {
      if (forceQueue) queue();
      return;
    }
    targetChanged = false;
    rects = nextRects;
    if (shapeChanged || sizeChanged) merged = new WeakMap();
    input = buttons.length
      ? { stamp: ++stamp, buttons, ...(backdrops.length ? { backdrops } : {}) }
      : undefined;
    queue();
  }

  function sync() {
    if (!snapshot) return;
    const result = presentation(snapshot, settings?.mutedSources, notifySubagentsEnabled());
    const now = performance.now();
    const taskKey = JSON.stringify([result.state, result.animationKey ?? '', result.reminderId ?? '']);
    const taskChanged = taskKey !== lastTaskKey;
    lastTaskKey = taskKey;
    const urgentTask =
      result.state === 'review' ||
      result.state === 'waiting' ||
      Boolean(result.animationKey && !completedAnimations.has(result.animationKey));
    if (taskChanged && urgentTask) {
      clearInteractionAnimation();
      clearAutonomousActivity();
    }
    if (validationState) {
      clearInteractionAnimation();
      clearAutonomousActivity();
      activeAnimation = undefined;
      pet.renderer.setState(validationState, now);
    } else if (!interactionStep && !autonomousActivity?.state) {
      if (result.animationKey && completedAnimations.has(result.animationKey)) {
        activeAnimation = undefined;
        pet.renderer.setState(result.fallbackState, now);
      } else if (result.animationKey) {
        const restarted =
          activeAnimation !== undefined &&
          activeAnimation.key !== result.animationKey &&
          activeAnimation.state === result.state;
        const applied = restarted
          ? pet.renderer.setState(result.state, now, true)
          : pet.renderer.setState(result.state, now);
        if (applied === false) {
          activeAnimation = undefined;
          pet.renderer.setState(result.fallbackState, now);
        } else {
          activeAnimation = {
            state: result.state,
            key: result.animationKey,
            fallbackState: result.fallbackState,
          };
        }
      } else {
        activeAnimation = undefined;
        pet.renderer.setState(result.state, now);
      }
    }
    renderStack();
  }

  async function preferences(value: Settings) {
    settings = value;
    const fps = validationFps === 30 || validationFps === 60 ? validationFps : value.fps;
    setFps(fps);
    pet.renderer.setFps(fps);
    sync();
    const petId = value.petId;
    if (selectedPet === petId) return;
    selectedPet = petId;
    const token = ++change;
    try {
      const asset = builtinPets.get(petId) ?? await api.asset(petId);
      if (token !== change || stopped) return;
      await pet.renderer.mount(canvas, asset.manifest, asset.atlasUrl);
      if (token !== change || stopped) return;
      petName = asset.manifest.name;
      canvas.setAttribute('aria-label', petName);
      petLoadErrorOrder = undefined;
      sync();
    } catch (e) {
      if (token !== change || stopped) return;
      await pet.mountDefault();
      if (token !== change || stopped) return;
      selectedPet = defaultPet.manifest.id;
      petName = defaultPet.manifest.name;
      canvas.setAttribute('aria-label', petName);
      console.error(e);
      petLoadErrorOrder = ++bubbleOrder;
      renderStack(true);
    }
  }

  function observe(value: PetSnapshot) {
    const previous = snapshot;
    snapshot = value;
    pruneImportedDismissed(value);
    sync();
    if (!previous || !settings?.sound) return;
    const known = new Set(previous.notifications.map((n) => n.id));
    const allowSubagents = notifySubagentsEnabled();
    const subagentSessions = allowSubagents
      ? undefined
      : new Set(value.sessions.filter(isSubagentSession).map((s) => keyId(s.key)));
    const freshList = value.notifications.filter(
      (n) =>
        !known.has(n.id) &&
        !n.read &&
        !settings!.mutedSources.includes(n.key.source) &&
        (!subagentSessions || !subagentSessions.has(keyId(n.key))) &&
        (n.kind.type === 'turn_ended' || !n.kind.resolved),
    );
    if (freshList.length > 0) {
      freshList.sort(
        (a, b) =>
          notificationSoundPriority(b) - notificationSoundPriority(a) ||
          b.occurredAtMs - a.occurredAtMs,
      );
      const chosen = freshList[0];
      void Promise.resolve(
        api.playAlert(chosen.key.source, alertCueForNotification(chosen)),
      ).catch(console.error);
    }
  }

  function subscribe(work: Promise<() => void>) {
    void work
      .then((stop) => {
        if (stopped) stop();
        else stops.push(stop);
      })
      .catch(console.error);
  }

  subscribe(
    observeSnapshot(
      api.settingsEvents,
      api.settings,
      (v) => {
        if (!stopped) void preferences(v).catch(console.error);
      },
      console.error,
    ),
  );
  subscribe(
    observeSnapshot(
      api.snapshots,
      api.snapshot,
      (v) => {
        if (!stopped) observe(v);
      },
      console.error,
    ),
  );
  subscribe(
    listen<PetClickEvent>('pet:click', (event) => {
      handlePetClick(event.payload);
    }),
  );
  subscribe(
    listen('pet:drag-end', () => {
      handleDragEnd();
    }),
  );
  subscribe(
    listen<PetActivityEvent>('pet:activity', (event) => {
      handleAutonomousActivity(event.payload);
    }),
  );
  subscribe(
    listen<{ stamp: number; bubble?: number; action: BubbleAction }>('pet:bubble-action', (event) => {
      const payload = event.payload;
      if (stopped || stackEl.hidden || input?.stamp !== payload.stamp || stamp !== payload.stamp) return;
      if (payload.action === 'close_all') {
        if (!closeAllButton.hidden && closeAllButton.parentNode === stackEl) closeAllButton.click();
        return;
      }
      const bubbleIndex = payload.bubble ?? 0;
      const targetBubble = stackEl.querySelector<HTMLElement>(
        `aside.bubble[data-bubble-index="${bubbleIndex}"]`,
      );
      if (!targetBubble || targetBubble.hidden) return;
      const button = Array.from(targetBubble.querySelectorAll<HTMLButtonElement>('button[data-action]')).find(
        (element) => element.dataset.action === payload.action,
      );
      if (button && !button.hidden) button.click();
    }),
  );

  return {
    pose(mask: AlphaMask, frame: FrameRect) {
      const sizeChanged =
        !measuredSize || measuredSize.width !== mask.width || measuredSize.height !== mask.height;
      base = { mask, frame };
      if (!stackEl.hidden && sizeChanged) handleBubbleLayoutChange(true);
      else queue();
    },
    click(event: PetClickEvent) {
      handlePetClick(event);
    },
    animationEnded(state: string, timestamp: number) {
      if (stopped || validationState) return;
      if (interactionStep && !interactionStep.timed && interactionStep.state === state) {
        playNextInteractionStep(timestamp);
        return;
      }
      if (autonomousActivity && autonomousActivity.state === state) {
        autonomousActivity.state = undefined;
        if (snapshot) sync();
        else pet.renderer.setState('idle', timestamp);
        return;
      }
      if (!activeAnimation || activeAnimation.state !== state) return;
      const finished = activeAnimation;
      activeAnimation = undefined;
      completedAnimations.add(finished.key);
      if (completedAnimations.size > MAX_DISMISSED) {
        const oldest = completedAnimations.values().next().value;
        if (oldest !== undefined) completedAnimations.delete(oldest);
      }
      pet.renderer.setState(finished.fallbackState, timestamp);
    },
    stop() {
      stopped = true;
      clearAllReminderAutoCloseTimers();
      clearAllEphemeralBubbles();
      clearInteractionAnimation();
      clearAutonomousActivity();
      observer.disconnect();
      stops.forEach((stop) => stop());
      views.clear();
      stackEl.remove();
    },
  };
}
