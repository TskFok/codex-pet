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
import { presentation, withBubble } from './reminder';

type BubbleAction = 'session' | 'application' | 'copy' | 'close' | InteractionBubbleAction;
type BubbleBackdrop = FrameRect & { radius: number };
type BubbleInput = {
  stamp: number;
  buttons: { action: BubbleAction; rect: FrameRect }[];
  backdrop?: BubbleBackdrop;
};
type Pose = { mask: AlphaMask; frame: FrameRect };
export type InputPose = Pose & { bubbleInput?: BubbleInput };
const PET_LOAD_ERROR_ID = 'pet-load-error';
const MAX_DISMISSED = 200;
const DEFAULT_BUBBLE_RADIUS = 14;
export function bindReminder(
  canvas: HTMLCanvasElement,
  pet: ReturnType<typeof mountPet>,
  sendPose: (pose: InputPose) => void,
  setFps: (fps: 30 | 60) => void,
  validationState?: string,
  validationFps?: number,
) {
  const bubble = document.createElement('aside');
  bubble.className = 'bubble';
  bubble.hidden = true;
  const header = document.createElement('div');
  header.className = 'bubble-header';
  const title = document.createElement('strong');
  const close = document.createElement('button');
  close.type = 'button';
  close.className = 'bubble-close';
  close.dataset.action = 'close';
  close.setAttribute('aria-label', '关闭提示框');
  close.title = '关闭';
  close.hidden = true;
  header.append(title, close);
  const detail = document.createElement('span');
  const controls = document.createElement('div');
  bubble.append(header, detail, controls);
  document.body.append(bubble);
  let base: Pose | undefined,
    rect: FrameRect | undefined,
    measuredSize: { width: number; height: number } | undefined,
    merged = new WeakMap<AlphaMask, AlphaMask>();
  let input: BubbleInput | undefined;
  let stamp = 0;
  let targetChanged = false;
  let target: string | undefined;
  let currentReminderId: string | undefined;
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
  let interactionBubble: InteractionBubble | undefined;
  let interactionBubbleTimer: ReturnType<typeof setTimeout> | undefined;
  let interactionQueue: InteractionStep[] = [];
  let interactionStep: { state: string; timed: boolean } | undefined;
  let interactionStepTimer: ReturnType<typeof setTimeout> | undefined;
  let autonomousActivity: { id: string; state?: string } | undefined;
  let lastTaskKey: string | undefined;
  function clearAutonomousActivity() {
    autonomousActivity = undefined;
  }
  function clearInteractionBubble() {
    if (interactionBubbleTimer !== undefined) {
      clearTimeout(interactionBubbleTimer);
      interactionBubbleTimer = undefined;
    }
    interactionBubble = undefined;
  }
  function clearInteractionAnimation() {
    if (interactionStepTimer !== undefined) {
      clearTimeout(interactionStepTimer);
      interactionStepTimer = undefined;
    }
    interactionQueue = [];
    interactionStep = undefined;
  }
  function hasActiveReminder(): boolean {
    if (currentReminderId === PET_LOAD_ERROR_ID) return true;
    if (!snapshot) return false;
    const result = presentation(snapshot, settings?.mutedSources);
    return Boolean(result.detail && !(result.reminderId && dismissed.has(result.reminderId)));
  }
  function updateInteractionControls(actions: InteractionBubbleAction[]) {
    if (actions.length === 0) {
      updateControls();
      return;
    }
    const id = `interaction:${actions.join(',')}`;
    if (id === target) return;
    target = id;
    ++stamp;
    targetChanged = true;
    const bar = document.createElement('div');
    bar.className = 'actions';
    for (const action of actions) {
      const button = document.createElement('button');
      button.type = 'button';
      button.dataset.action = action;
      button.textContent = INTERACTION_ACTION_LABELS[action];
      button.onclick = () => {
        if (stopped) return;
        const now = performance.now();
        applyInteraction(interactions.handleAction(action, now, { petName, snapshot }), now);
      };
      bar.append(button);
    }
    controls.replaceChildren(bar);
  }
  function renderInteractionBubble() {
    if (!interactionBubble || hasActiveReminder()) return;
    currentReminderId = undefined;
    bubble.hidden = false;
    close.hidden = false;
    close.textContent = '×';
    updateInteractionControls(interactionBubble.actions);
    title.textContent = interactionBubble.title;
    detail.textContent = interactionBubble.detail;
    measure();
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
  function applyInteraction(outcome: InteractionOutcome | undefined, nowMs: number) {
    if (!outcome || stopped) return;
    if (outcome.effect) {
      pet.renderer.triggerInteractionEffect?.(outcome.effect, nowMs);
    }
    if (outcome.openNotifications) {
      void api.openNotifications().catch(console.error);
    }
    if (outcome.closeBubble) {
      clearInteractionBubble();
      if (!hasActiveReminder()) {
        ++stamp;
        targetChanged = true;
        hideBubble();
        measure();
      }
    }
    if (!validationState && outcome.steps && outcome.steps.length > 0) {
      activeAnimation = undefined;
      interactionQueue = [...outcome.steps];
      playNextInteractionStep(nowMs);
    }
    if (outcome.bubble && !hasActiveReminder()) {
      clearInteractionBubble();
      interactionBubble = outcome.bubble;
      const delayMs = Math.max(0, outcome.bubble.expiresAtMs - nowMs);
      interactionBubbleTimer = setTimeout(() => {
        interactionBubbleTimer = undefined;
        if (stopped || interactionBubble?.id !== outcome.bubble!.id) return;
        interactionBubble = undefined;
        if (!hasActiveReminder()) {
          ++stamp;
          targetChanged = true;
          hideBubble();
          measure();
        }
      }, delayMs);
      ++stamp;
      targetChanged = true;
      renderInteractionBubble();
    }
  }
  function handlePetClick(event: PetClickEvent) {
    if (stopped) return;
    if (autonomousActivity) {
      clearAutonomousActivity();
      void api.interruptPetActivity().catch(() => {});
    }
    const now = performance.now();
    applyInteraction(interactions.handleClick(event, now, { petName, snapshot }), now);
  }
  function handleDragEnd() {
    clearAutonomousActivity();
    if (stopped || validationState || hasActiveReminder()) return;
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
    if (interactionStep || hasActiveReminder()) return;
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
    if (event.thought && !hasActiveReminder() && !interactionBubble) {
      const bubbleId = `pet-activity:${event.id}`;
      const durationMs = Math.max(1200, Math.min(8000, event.thoughtDurationMs ?? 3600));
      clearInteractionBubble();
      interactionBubble = {
        id: bubbleId,
        title: petName,
        detail: event.thought,
        actions: [],
        expiresAtMs: now + durationMs,
      };
      interactionBubbleTimer = setTimeout(() => {
        interactionBubbleTimer = undefined;
        if (stopped || interactionBubble?.id !== bubbleId) return;
        interactionBubble = undefined;
        if (!hasActiveReminder()) {
          ++stamp;
          targetChanged = true;
          hideBubble();
          measure();
        }
      }, durationMs);
      ++stamp;
      targetChanged = true;
      renderInteractionBubble();
    }
  }
  function queue() {
    if (!base) return;
    const mask = base.mask,
      frame = base.frame;
    if (!rect) {
      sendPose(base);
      return;
    }
    const shape = merged.get(mask) ?? withBubble(mask, rect);
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
    const showing = !bubble.hidden && !!c.width && !!c.height && targetWidth > 0 && targetHeight > 0;
    const bubbleBox = showing ? bubble.getBoundingClientRect() : undefined;
    const next = bubbleBox ? bounds(bubbleBox) : undefined;
    const backdropRect = bubbleBox ? clampRect(bubbleBox) : undefined;
    const cssRadius =
      showing && typeof getComputedStyle === 'function'
        ? Number.parseFloat(getComputedStyle(bubble).borderTopLeftRadius)
        : Number.NaN;
    const scale = Math.min(sx, sy);
    const backdrop: BubbleBackdrop | undefined =
      backdropRect && backdropRect.width > 0 && backdropRect.height > 0
        ? {
            ...backdropRect,
            radius: Math.min(
              Math.max(0, (Number.isFinite(cssRadius) ? cssRadius : DEFAULT_BUBBLE_RADIUS) * scale),
              Math.min(backdropRect.width, backdropRect.height) / 2,
            ),
          }
        : undefined;
    const buttons = showing
      ? Array.from(bubble.querySelectorAll<HTMLButtonElement>('button[data-action]:not([hidden])'))
          .map((button) => ({
            action: button.dataset.action as BubbleAction,
            rect: clampRect(button.getBoundingClientRect()),
          }))
          .filter((button) => button.rect.width > 0 && button.rect.height > 0)
      : [];
    const sizeChanged =
      base !== undefined &&
      (!measuredSize || measuredSize.width !== targetWidth || measuredSize.height !== targetHeight);
    const shapeChanged = JSON.stringify(next) !== JSON.stringify(rect);
    const backdropChanged = JSON.stringify(backdrop) !== JSON.stringify(input?.backdrop);
    const buttonsChanged = JSON.stringify(buttons) !== JSON.stringify(input?.buttons ?? []);
    if (base) measuredSize = { width: targetWidth, height: targetHeight };
    if (!shapeChanged && !backdropChanged && !buttonsChanged && !targetChanged && !sizeChanged) {
      if (forceQueue) queue();
      return;
    }
    targetChanged = false;
    rect = next;
    if (shapeChanged || sizeChanged) merged = new WeakMap();
    input = buttons.length ? { stamp: ++stamp, buttons, ...(backdrop ? { backdrop } : {}) } : undefined;
    queue();
  }
  function updateControls(next?: SessionKey) {
    const id = next && keyId(next);
    if (id === target) return;
    target = id;
    ++stamp;
    targetChanged = true;
    controls.replaceChildren(
      ...(next
        ? [
            createNavigation(
              document,
              next,
              { open: api.openSession, copy: api.copySession },
              true,
              () => measure(),
            ),
          ]
        : []),
    );
  }
  function hideBubble() {
    bubble.hidden = true;
    close.hidden = true;
    close.textContent = '';
    title.textContent = '';
    detail.textContent = '';
    updateControls();
  }
  function dismiss() {
    if (interactionBubble && !currentReminderId) {
      clearInteractionBubble();
      ++stamp;
      targetChanged = true;
      hideBubble();
      measure();
      return;
    }
    if (currentReminderId && currentReminderId !== PET_LOAD_ERROR_ID) {
      dismissed.add(currentReminderId);
      if (dismissed.size > MAX_DISMISSED) {
        const oldest = dismissed.values().next().value;
        if (oldest !== undefined) dismissed.delete(oldest);
      }
    }
    currentReminderId = undefined;
    ++stamp;
    targetChanged = true;
    hideBubble();
    measure();
  }
  close.onclick = dismiss;
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
    if (!result.detail || (result.reminderId && dismissed.has(result.reminderId))) {
      if (currentReminderId !== PET_LOAD_ERROR_ID) {
        currentReminderId = undefined;
        if (interactionBubble) {
          renderInteractionBubble();
        } else {
          hideBubble();
          measure();
        }
      }
      return;
    }
    clearInteractionBubble();
    if (currentReminderId !== result.reminderId) {
      currentReminderId = result.reminderId;
      ++stamp;
      targetChanged = true;
    }
    bubble.hidden = false;
    close.hidden = false;
    close.textContent = '×';
    updateControls(result.key);
    title.textContent = result.title || '';
    detail.textContent = result.detail;
    measure();
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
        if (currentReminderId === PET_LOAD_ERROR_ID) currentReminderId = undefined;
        sync();
      }
    } catch (e) {
      if (token !== change || stopped) return;
      selectedPet = 'yueying-cape-dancer';
      petName = '月影';
      await pet.mountDefault();
      console.error(e);
      clearInteractionBubble();
      currentReminderId = PET_LOAD_ERROR_ID;
      ++stamp;
      targetChanged = true;
      updateControls();
      title.textContent = '无法加载所选桌宠';
      detail.textContent = '请在设置中重新选择宠物';
      close.textContent = '×';
      close.hidden = false;
      bubble.hidden = false;
      measure();
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
    listen<{ stamp: number; action: BubbleAction }>('pet:bubble-action', (event) => {
      const payload = event.payload;
      if (stopped || bubble.hidden || input?.stamp !== payload.stamp || stamp !== payload.stamp) return;
      const button = Array.from(bubble.querySelectorAll<HTMLButtonElement>('button[data-action]')).find(
        (element) => element.dataset.action === payload.action,
      );
      if (button && !button.hidden) button.click();
    }),
  );
  const observer = new ResizeObserver(() => measure());
  observer.observe(canvas);
  observer.observe(bubble);
  return {
    pose(mask: AlphaMask, frame: FrameRect) {
      const sizeChanged =
        !measuredSize || measuredSize.width !== mask.width || measuredSize.height !== mask.height;
      base = { mask, frame };
      if (!bubble.hidden && sizeChanged) measure(true);
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
      clearInteractionBubble();
      clearInteractionAnimation();
      clearAutonomousActivity();
      observer.disconnect();
      stops.forEach((stop) => stop());
      bubble.remove();
    },
  };
}
