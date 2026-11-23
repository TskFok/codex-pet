import { invoke } from '@tauri-apps/api/core';
import { api, observeSnapshot } from '../api';
import type { AlphaMask } from '../animation/hit-mask';
import type { FrameRect, PetSnapshot, Settings } from '../types';
import type { mountPet } from './pet';
import { presentation, withBubble } from './reminder';

type Pose = { mask: AlphaMask; frame: FrameRect };
export function bindReminder(
  canvas: HTMLCanvasElement,
  pet: ReturnType<typeof mountPet>,
  sendPose: (pose: Pose) => void,
  setFps: (fps: 30 | 60) => void,
  validationState?: string,
  validationFps?: number,
) {
  const bubble = document.createElement('aside');
  bubble.className = 'bubble';
  bubble.hidden = true;
  const title = document.createElement('strong'),
    detail = document.createElement('span');
  bubble.append(title, detail);
  document.body.append(bubble);
  let base: Pose | undefined,
    rect: FrameRect | undefined,
    merged = new WeakMap<AlphaMask, AlphaMask>();
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
    sendPose({ mask: shape, frame });
  }
  function measure() {
    const b = bubble.getBoundingClientRect(),
      c = canvas.getBoundingClientRect();
    const sx = canvas.width / c.width,
      sy = canvas.height / c.height;
    const next =
      !bubble.hidden && c.width && c.height
        ? { x: (b.left - c.left) * sx, y: (b.top - c.top) * sy, width: b.width * sx, height: b.height * sy }
        : undefined;
    if (JSON.stringify(next) === JSON.stringify(rect)) return;
    rect = next;
    merged = new WeakMap();
    queue();
  }
  function sync() {
    if (!snapshot) return;
    const result = presentation(snapshot, settings?.mutedSources);
    pet.renderer.setState(validationState || result.state, performance.now());
    bubble.hidden = !result.detail;
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
  const observer = new ResizeObserver(measure);
  observer.observe(canvas);
  observer.observe(bubble);
  return {
    pose(mask: AlphaMask, frame: FrameRect) {
      base = { mask, frame };
      queue();
    },
    stop() {
      stopped = true;
      observer.disconnect();
      stops.forEach((stop) => stop());
      bubble.remove();
    },
  };
}
