import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { api, observeSnapshot } from '../api';
import type { AlphaMask } from '../animation/hit-mask';
import type { FrameRect, PetSnapshot, SessionKey, Settings } from '../types';
import type { mountPet } from './pet';
import { createNavigation } from './navigation';
import { keyId } from './notifications';
import { presentation, withBubble } from './reminder';

type BubbleAction = 'session' | 'application' | 'copy';
type BubbleInput = { stamp: number; buttons: { action: BubbleAction; rect: FrameRect }[] };
type Pose = { mask: AlphaMask; frame: FrameRect };
export type InputPose = Pose & { bubbleInput?: BubbleInput };
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
  const title = document.createElement('strong'),
    detail = document.createElement('span');
  const controls = document.createElement('div');
  bubble.append(title, detail, controls);
  document.body.append(bubble);
  let base: Pose | undefined,
    rect: FrameRect | undefined,
    measuredSize: { width: number; height: number } | undefined,
    merged = new WeakMap<AlphaMask, AlphaMask>();
  let input: BubbleInput | undefined;
  let stamp = 0;
  let targetChanged = false;
  let target: string | undefined;
  let snapshot: PetSnapshot | undefined,
    settings: Settings | undefined,
    selectedPet = 'yueying-cape-dancer';
  let change = 0,
    stopped = false;
  const stops: (() => void)[] = [];
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
    const clampButton = (b: DOMRect): FrameRect => {
      const raw = bounds(b);
      const x1 = Math.max(0, Math.min(targetWidth, raw.x));
      const y1 = Math.max(0, Math.min(targetHeight, raw.y));
      const x2 = Math.max(0, Math.min(targetWidth, raw.x + raw.width));
      const y2 = Math.max(0, Math.min(targetHeight, raw.y + raw.height));
      return { x: x1, y: y1, width: x2 - x1, height: y2 - y1 };
    };
    const showing = !bubble.hidden && !!c.width && !!c.height && targetWidth > 0 && targetHeight > 0;
    const next = showing ? bounds(bubble.getBoundingClientRect()) : undefined;
    const buttons = showing
      ? Array.from(controls.querySelectorAll<HTMLButtonElement>('button:not([hidden])'))
          .map((button) => ({
            action: button.dataset.action as BubbleAction,
            rect: clampButton(button.getBoundingClientRect()),
          }))
          .filter((button) => button.rect.width > 0 && button.rect.height > 0)
      : [];
    const sizeChanged =
      base !== undefined &&
      (!measuredSize || measuredSize.width !== targetWidth || measuredSize.height !== targetHeight);
    const shapeChanged = JSON.stringify(next) !== JSON.stringify(rect);
    const buttonsChanged = JSON.stringify(buttons) !== JSON.stringify(input?.buttons ?? []);
    if (base) measuredSize = { width: targetWidth, height: targetHeight };
    if (!shapeChanged && !buttonsChanged && !targetChanged && !sizeChanged) {
      if (forceQueue) queue();
      return;
    }
    targetChanged = false;
    rect = next;
    if (shapeChanged || sizeChanged) merged = new WeakMap();
    input = buttons.length ? { stamp: ++stamp, buttons } : undefined;
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
  function sync() {
    if (!snapshot) return;
    const result = presentation(snapshot, settings?.mutedSources);
    pet.renderer.setState(validationState || result.state, performance.now());
    bubble.hidden = !result.detail;
    updateControls(result.key);
    title.textContent = result.title || '';
    detail.textContent = result.detail || '';
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
        canvas.setAttribute('aria-label', '月影');
      } else {
        const asset = await api.asset(selectedPet);
        if (token !== change || stopped) return;
        await pet.renderer.mount(canvas, asset.manifest, asset.atlasUrl);
        if (token === change && !stopped) canvas.setAttribute('aria-label', asset.manifest.name);
      }
      if (token === change && !stopped) sync();
    } catch (e) {
      if (token !== change || stopped) return;
      selectedPet = 'yueying-cape-dancer';
      await pet.mountDefault();
      console.error(e);
      updateControls();
      title.textContent = '无法加载所选桌宠';
      detail.textContent = '请在设置中重新选择宠物';
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
    listen<{ stamp: number; action: BubbleAction }>('pet:bubble-action', (event) => {
      const payload = event.payload;
      if (stopped || bubble.hidden || input?.stamp !== payload.stamp || stamp !== payload.stamp) return;
      const button = Array.from(controls.querySelectorAll<HTMLButtonElement>('button')).find(
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
    stop() {
      stopped = true;
      observer.disconnect();
      stops.forEach((stop) => stop());
      bubble.remove();
    },
  };
}
