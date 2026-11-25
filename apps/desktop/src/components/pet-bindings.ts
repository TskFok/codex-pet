import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, observeSnapshot } from '../api';
import type { AlphaMask } from '../animation/hit-mask';
import type { FrameRect, PetActivityEvent, PetSnapshot, SessionKey, Settings } from '../types';
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
import { presentation, withBubbles } from './reminder';

type BubbleAction = 'session' | 'application' | 'copy' | 'close' | InteractionBubbleAction;
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
const MAX_DISMISSED = 200;
const DEFAULT_BUBBLE_RADIUS = 14;
const MAX_VISIBLE_BUBBLES = 4;
const MAX_EPHEMERAL_BUBBLES = 3;
const INTERACTION_REPLACE_WINDOW_MS = 450;

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
  const observer = new ResizeObserver(() => measure());
  observer.observe(canvas);
  observer.observe(stackEl);

  let base: Pose | undefined,
    rects: FrameRect[] = [],
    measuredSize: { width: number; height: number } | undefined,
    merged = new WeakMap<AlphaMask, AlphaMask>();
  let input: BubbleInput | undefined;
  let stamp = 0;
  let targetChanged = false;
  let bubbleOrder = 0;
  let renderedIds: string[] = [];
  const views = new Map<string, BubbleView>();
  const reminderOrder = new Map<string, number>();
  let petLoadErrorOrder: number | undefined;
  const ephemeralBubbles: EphemeralBubble[] = [];

  const dismissed = new Set<string>();
  const completedAnimations = new Set<string>();
  let activeAnimation: { state: string; key: string; fallbackState: string } | undefined;
  let snapshot: PetSnapshot | undefined,
    settings: Settings | undefined,
    selectedPet = 'yueying-cape-dancer',
    petName = '月影';
  let change = 0,
    stopped = false;
  const stops: (() => void)[] = [];
  const interactions = new PetInteractionController();
  let interactionQueue: InteractionStep[] = [];
  let interactionStep: { state: string; timed: boolean } | undefined;
  let interactionStepTimer: ReturnType<typeof setTimeout> | undefined;
  let autonomousActivity: { id: string; state?: string } | undefined;
  let lastTaskKey: string | undefined;

  function clearAutonomousActivity() {
    autonomousActivity = undefined;
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

  function hasUrgentAttention(): boolean {
    return Boolean(snapshot?.sessions.some((session) => session.status === 'needs_attention'));
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
    applyInteraction(interactions.handleClick(event, now, { petName, snapshot }), now, { fromClick: true });
  }

  function handleDragEnd() {
    clearAutonomousActivity();
    if (stopped || validationState || hasUrgentAttention()) return;
    const now = performance.now();
    applyInteraction(interactions.handleDragEnd(now, { petName, snapshot }), now);
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
  }

  function computeVisibleBubbles(): RenderBubble[] {
    const entries: RenderBubble[] = [];
    const activeReminderIds = new Set<string>();
    if (snapshot) {
      const result = presentation(snapshot, settings?.mutedSources);
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
    if (entries.length <= MAX_VISIBLE_BUBBLES) {
      entries.sort((a, b) => a.order - b.order);
      return entries;
    }
    // 超过同屏上限时，优先保留错误与待审批/待回答提醒，其余保留最新叠加的气泡
    const pinned = entries.filter((entry) => entry.rank >= 5).sort((a, b) => b.order - a.order);
    const others = entries.filter((entry) => entry.rank < 5).sort((a, b) => b.order - a.order);
    const kept = [...pinned.slice(0, MAX_VISIBLE_BUBBLES)];
    const remaining = MAX_VISIBLE_BUBBLES - kept.length;
    if (remaining > 0) kept.push(...others.slice(0, remaining));
    kept.sort((a, b) => a.order - b.order);
    return kept;
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

  function dismissBubble(id: string, kind: BubbleKind) {
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
    dismissed.add(id);
    if (dismissed.size > MAX_DISMISSED) {
      const oldest = dismissed.values().next().value;
      if (oldest !== undefined) dismissed.delete(oldest);
    }
    reminderOrder.delete(id);
    renderStack(true);
  }

  function renderStack(forceInvalidate = false) {
    const visible = computeVisibleBubbles();
    const visibleIds = visible.map((entry) => entry.id);
    const visibleSet = new Set(visibleIds);
    let invalidated =
      forceInvalidate ||
      visibleIds.length !== renderedIds.length ||
      visibleIds.some((id, index) => renderedIds[index] !== id);

    for (const pair of Array.from(views.entries())) {
      const id = pair[0];
      const view = pair[1];
      if (!visibleSet.has(id)) {
        observer.unobserve?.(view.element);
        view.element.remove();
        views.delete(id);
        invalidated = true;
      }
    }

    for (let index = 0; index < visible.length; index++) {
      const entry = visible[index];
      let view = views.get(entry.id);
      if (!view) {
        view = createBubbleView(entry.id);
        views.set(entry.id, view);
        invalidated = true;
      }
      view.element.hidden = false;
      view.element.dataset.bubbleIndex = String(index);
      view.element.dataset.bubbleKind = entry.kind;
      view.title.textContent = entry.title;
      view.detail.textContent = entry.detail;
      view.close.hidden = false;
      view.close.textContent = '×';
      view.close.onclick = () => dismissBubble(entry.id, entry.kind);

      if (entry.kind === 'reminder' && entry.key) {
        const controlsKey = `nav:${keyId(entry.key)}`;
        view.element.classList.toggle('has-actions', true);
        if (view.controlsKey !== controlsKey) {
          view.controlsKey = controlsKey;
          invalidated = true;
          view.controls.replaceChildren(
            createNavigation(
              document,
              entry.key,
              { open: api.openSession, copy: api.copySession },
              true,
              () => measure(),
            ),
          );
        }
      } else if (entry.actions && entry.actions.length > 0) {
        const controlsKey = `interaction:${entry.actions.join(',')}`;
        view.element.classList.toggle('has-actions', true);
        if (view.controlsKey !== controlsKey) {
          view.controlsKey = controlsKey;
          invalidated = true;
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
              applyInteraction(interactions.handleAction(action, now, { petName, snapshot }), now, {
                fromAction: true,
                sourceBubbleId: entry.id,
              });
            };
            bar.append(button);
          }
          view.controls.replaceChildren(bar);
        }
      } else {
        view.element.classList.toggle('has-actions', false);
        if (view.controlsKey !== undefined) {
          view.controlsKey = undefined;
          invalidated = true;
          view.controls.replaceChildren();
        }
      }
    }

    const desiredNodes = visible.map((entry) => views.get(entry.id)!.element);
    const orderMatches =
      stackEl.children.length === desiredNodes.length &&
      desiredNodes.every((node, index) => stackEl.children[index] === node);
    if (!orderMatches) {
      stackEl.replaceChildren(...desiredNodes);
    }

    stackEl.hidden = visible.length === 0;
    stackEl.classList.toggle('is-stacked', visible.length > 1);
    renderedIds = visibleIds;
    if (invalidated) {
      ++stamp;
      targetChanged = true;
    }
    measure();
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
    const result = presentation(snapshot, settings?.mutedSources);
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
    if (selectedPet === value.petId) return;
    selectedPet = value.petId;
    const token = ++change;
    try {
      if (selectedPet === 'yueying-cape-dancer') {
        await pet.mountDefault();
        petName = '月影';
        canvas.setAttribute('aria-label', '月影');
      } else {
        const asset = await api.asset(selectedPet);
        if (token !== change || stopped) return;
        await pet.renderer.mount(canvas, asset.manifest, asset.atlasUrl);
        if (token === change && !stopped) {
          petName = asset.manifest.name;
          canvas.setAttribute('aria-label', asset.manifest.name);
        }
      }
      if (token === change && !stopped) {
        if (petLoadErrorOrder !== undefined) petLoadErrorOrder = undefined;
        sync();
      }
    } catch (e) {
      if (token !== change || stopped) return;
      selectedPet = 'yueying-cape-dancer';
      petName = '月影';
      await pet.mountDefault();
      console.error(e);
      petLoadErrorOrder = ++bubbleOrder;
      renderStack(true);
    }
  }

  function observe(value: PetSnapshot) {
    const previous = snapshot;
    snapshot = value;
    sync();
    if (!previous || !settings?.sound) return;
    const known = new Set(previous.notifications.map((n) => n.id));
    const fresh = value.notifications.find(
      (n) =>
        !known.has(n.id) &&
        !n.read &&
        !settings!.mutedSources.includes(n.key.source) &&
        (n.kind.type === 'turn_ended' || !n.kind.resolved),
    );
    if (fresh) void invoke('play_alert', { source: fresh.key.source }).catch(console.error);
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
      if (!stackEl.hidden && sizeChanged) measure(true);
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
