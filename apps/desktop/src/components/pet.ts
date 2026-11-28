import type { FrameRect } from '../types';
import { PetRenderer } from '../animation/renderer';
import type { AnimationSample } from '../animation/clock';
import { defaultPet } from '../assets/builtin-pets';

import type { AlphaMask } from '../animation/hit-mask';
import type { PetClickEvent } from './pet-interactions';

export interface PlaybackSignal {
  revision: number;
  visible: boolean;
  suspended: boolean;
}

export class PetVisibility {
  private signal: PlaybackSignal = { revision: -1, visible: false, suspended: false };
  private documentVisible = true;
  private playing = false;

  constructor(
    private readonly setVisible: (visible: boolean, nowMs: number) => void,
    private readonly breakContinuity: () => void,
  ) {}

  update(signal: PlaybackSignal, nowMs: number): void {
    if (signal.revision <= this.signal.revision) return;
    const missedTransition = this.signal.revision >= 0 && signal.revision > this.signal.revision + 1;
    this.signal = signal;
    if (this.playing && missedTransition) this.updatePlaying(false, nowMs);
    this.sync(nowMs);
  }

  setDocumentVisible(visible: boolean, nowMs: number): void {
    this.documentVisible = visible;
    this.sync(nowMs);
  }

  private sync(nowMs: number): void {
    this.updatePlaying(this.documentVisible && this.signal.visible && !this.signal.suspended, nowMs);
  }

  private updatePlaying(playing: boolean, nowMs: number): void {
    if (this.playing === playing) return;
    this.playing = playing;
    if (!playing) this.breakContinuity();
    this.setVisible(playing, nowMs);
  }
}

export const PET_WINDOW_WIDTH = 240;
export const PET_WINDOW_HEIGHT = 440;
export const PET_STAGE_WIDTH = 160;
export const PET_STAGE_HEIGHT = 180;

/** 桌宠绘制区域固定位于窗口底部居中，上方保留给提示框避免遮挡。 */
export function petViewport(width: number, height: number): FrameRect {
  const stageWidth = (width * PET_STAGE_WIDTH) / PET_WINDOW_WIDTH;
  const stageHeight = (height * PET_STAGE_HEIGHT) / PET_WINDOW_HEIGHT;
  return {
    x: (width - stageWidth) / 2,
    y: height - stageHeight,
    width: stageWidth,
    height: stageHeight,
  };
}

export function mountPet(
  canvas: HTMLCanvasElement,
  options: {
    state?: string;
    fps: 30 | 60;
    staticFrame: boolean;
    onPose: (mask: AlphaMask, frame: FrameRect) => void;
    onDraw: (timestamp: number, state: string, sample: AnimationSample) => void;
    onAnimationEnd?: (state: string, timestamp: number) => void;
  },
): { renderer: PetRenderer; ready: Promise<void>; mountDefault: () => Promise<void> } {
  const manifest = defaultPet.manifest;
  const atlasUrl = defaultPet.atlasUrl;
  canvas.width = Math.round(canvas.clientWidth * window.devicePixelRatio);
  canvas.height = Math.round(canvas.clientHeight * window.devicePixelRatio);
  canvas.setAttribute('aria-label', manifest.name);
  // 图集 URL 在应用入口绑定，清单中的 atlas.path 仍是包内的相对路径。
  const renderer = new PetRenderer({ ...options, resolveAtlasUrl: () => atlasUrl, viewport: petViewport });
  const mountDefault = () => renderer.mount(canvas, manifest, atlasUrl);
  const ready = mountDefault();
  renderer.setFps(options.fps);
  if (options.state) renderer.setState(options.state, performance.now());
  return { renderer, ready, mountDefault };
}

const WINDOWS_DRAG_THRESHOLD_PX = 4;
const WINDOWS_MULTI_CLICK_MS = 350;
const WINDOWS_MULTI_CLICK_PX = 12;

export function bindPetInput(
  target: Pick<EventTarget, 'addEventListener' | 'removeEventListener'>,
  canvas: Pick<EventTarget, 'addEventListener' | 'removeEventListener'>,
  os: string,
  invokeCommand: (command: string) => Promise<unknown>,
  onPetClick?: (event: PetClickEvent) => void,
): () => void {
  let active:
    | {
        pointerId: number;
        startX: number;
        startY: number;
        altKey: boolean;
        shiftKey: boolean;
        dragged: boolean;
      }
    | undefined;
  let lastClick: { atMs: number; x: number; y: number; count: number } | undefined;

  const pointerDown = (event: Event) => {
    if (os !== 'windows') return;
    const pointer = event as PointerEvent;
    if (pointer.button !== 0) return;
    active = {
      pointerId: pointer.pointerId ?? 0,
      startX: pointer.clientX ?? 0,
      startY: pointer.clientY ?? 0,
      altKey: Boolean(pointer.altKey),
      shiftKey: Boolean(pointer.shiftKey),
      dragged: false,
    };
  };
  const pointerMove = (event: Event) => {
    if (os !== 'windows' || !active || active.dragged) return;
    const pointer = event as PointerEvent;
    if ((pointer.pointerId ?? 0) !== active.pointerId) return;
    const distance = Math.hypot((pointer.clientX ?? 0) - active.startX, (pointer.clientY ?? 0) - active.startY);
    if (distance >= WINDOWS_DRAG_THRESHOLD_PX) {
      active.dragged = true;
      void invokeCommand('pointer_input').catch(console.error);
    }
  };
  const pointerUp = (event: Event) => {
    if (os !== 'windows' || !active) return;
    const pointer = event as PointerEvent;
    if ((pointer.pointerId ?? 0) !== active.pointerId) return;
    const current = active;
    active = undefined;
    if (current.dragged || !onPetClick) return;
    const clientX = pointer.clientX ?? current.startX;
    const clientY = pointer.clientY ?? current.startY;
    const now = typeof event.timeStamp === 'number' && event.timeStamp > 0 ? event.timeStamp : performance.now();
    const isMulti =
      lastClick !== undefined &&
      now - lastClick.atMs <= WINDOWS_MULTI_CLICK_MS &&
      Math.hypot(clientX - lastClick.x, clientY - lastClick.y) <= WINDOWS_MULTI_CLICK_PX;
    const clickCount = isMulti ? lastClick!.count + 1 : 1;
    lastClick = { atMs: now, x: clientX, y: clientY, count: clickCount };
    const rect =
      typeof (canvas as Partial<HTMLElement>).getBoundingClientRect === 'function'
        ? (canvas as HTMLElement).getBoundingClientRect()
        : undefined;
    const x = rect && rect.width > 0 ? Math.max(0, Math.min(1, (clientX - rect.left) / rect.width)) : 0.5;
    const y = rect && rect.height > 0 ? Math.max(0, Math.min(1, (clientY - rect.top) / rect.height)) : 0.72;
    onPetClick({
      x,
      y,
      clickCount,
      altKey: current.altKey,
      shiftKey: current.shiftKey,
    });
  };
  const pointerCancel = () => {
    active = undefined;
  };
  const contextMenu = (event: Event) => {
    event.preventDefault();
    if (os === 'windows') {
      void invokeCommand('show_pet_menu').catch(console.error);
    }
  };
  canvas.addEventListener('pointerdown', pointerDown);
  target.addEventListener('pointermove', pointerMove);
  target.addEventListener('pointerup', pointerUp);
  target.addEventListener('pointercancel', pointerCancel);
  target.addEventListener('contextmenu', contextMenu);
  return () => {
    active = undefined;
    canvas.removeEventListener('pointerdown', pointerDown);
    target.removeEventListener('pointermove', pointerMove);
    target.removeEventListener('pointerup', pointerUp);
    target.removeEventListener('pointercancel', pointerCancel);
    target.removeEventListener('contextmenu', contextMenu);
  };
}
