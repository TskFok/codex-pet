import atlasUrl from '../../../../pet-output/yueying-cape-dancer/package/spritesheet.webp?url';
import type { PetManifest } from '../types';
import { PetRenderer } from '../animation/renderer';
import type { AnimationSample } from '../animation/clock';
import defaultManifest from '../assets/yueying.json';

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

export function mountPet(
  canvas: HTMLCanvasElement,
  options: {
    state?: string;
    fps: 30 | 60;
    staticFrame: boolean;
    onDraw: (timestamp: number, state: string, sample: AnimationSample) => void;
  },
): { renderer: PetRenderer; ready: Promise<void> } {
  if (defaultManifest.version !== 1) throw new Error('不支持此默认桌宠清单版本');
  const manifest: PetManifest = { ...defaultManifest, version: defaultManifest.version };
  canvas.width = Math.round(canvas.width * window.devicePixelRatio);
  canvas.height = Math.round(canvas.height * window.devicePixelRatio);
  // 图集 URL 在应用入口绑定，清单中的 atlas.path 仍是包内的相对路径。
  const renderer = new PetRenderer({ ...options, resolveAtlasUrl: () => atlasUrl });
  const ready = renderer.mount(canvas, manifest);
  renderer.setFps(options.fps);
  if (options.state) renderer.setState(options.state, performance.now());
  return { renderer, ready };
}
