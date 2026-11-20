import { FrameCadence } from '../diagnostics/frame-cadence';
import type { Animation, Atlas, FrameRect, PetManifest } from '../types';
import { AnimationClock, type AnimationSample } from './clock';
import { PoseMasks, type AlphaMask } from './hit-mask';

interface RendererOptions {
  onPose?: (mask: AlphaMask, frame: FrameRect) => void;
  resolveAtlasUrl: (atlas: Atlas) => string;
  staticFrame?: boolean;
  onDraw?: (timestamp: number, state: string, sample: AnimationSample) => void;
}

interface Mount {
  canvas: HTMLCanvasElement;
  context: CanvasRenderingContext2D;
  manifest: PetManifest;
  ready: Promise<void>;
  masks: PoseMasks;
  image?: HTMLImageElement;
}

export class PetRenderer {
  private readonly clock = new AnimationClock();
  private readonly atlases = new Map<string, Promise<HTMLImageElement>>();
  private mounted?: Mount;
  private visible = false;
  private state = 'idle';
  private animationStartedAt = 0;
  private frame?: number;
  private fps: 30 | 60 = 30;
  private cadence = new FrameCadence();

  constructor(private readonly options: RendererOptions) {}

  mount(canvas: HTMLCanvasElement, manifest: PetManifest): Promise<void> {
    if (this.mounted?.canvas === canvas && this.mounted.manifest === manifest) {
      return this.mounted.ready;
    }

    const context = canvas.getContext('2d');
    if (!context) return Promise.reject(new Error('无法创建桌宠 Canvas 绘图上下文'));
    this.cancelFrame();

    if (!this.animation(manifest, this.state)) {
      this.state = this.animation(manifest, 'idle') ? 'idle' : Object.keys(manifest.animations)[0];
    }
    this.animationStartedAt = performance.now();
    this.cadence = new FrameCadence(this.fps);
    const ready = this.atlas(this.options.resolveAtlasUrl(manifest.atlas)).then((image) => {
      if (this.mounted !== mount) return;
      mount.image = image;
      this.scheduleFrame();
    }).catch((error: unknown) => {
      if (this.mounted !== mount) return;
      this.mounted = undefined;
      throw error;
    });
    const mount: Mount = { canvas, context, manifest, ready, masks: new PoseMasks() };
    this.mounted = mount;
    return ready;
  }

  resize(width: number, height: number): void {
    const canvas = this.mounted?.canvas;
    if (!canvas || (width === canvas.width && height === canvas.height)) return;
    canvas.width = width;
    canvas.height = height;
    this.scheduleFrame();
  }

  setState(state: string, nowMs: number): void {
    if (state === this.state || !this.animation(this.mounted?.manifest, state)) return;
    this.state = state;
    this.animationStartedAt = nowMs;
    if (this.options.staticFrame) this.scheduleFrame();
  }

  setVisible(visible: boolean, nowMs: number): void {
    if (this.visible === visible) return;
    this.visible = visible;
    if (!visible) {
      this.cancelFrame();
      return;
    }
    this.animationStartedAt = nowMs;
    this.cadence = new FrameCadence(this.fps);
    this.scheduleFrame();
  }

  setFps(fps: 30 | 60): void {
    if (fps === this.fps) return;
    this.fps = fps;
    this.cadence = new FrameCadence(fps);
  }

  dispose(): void {
    this.cancelFrame();
    this.mounted = undefined;
    this.visible = false;
    this.state = 'idle';
    this.atlases.clear();
  }

  private animation(manifest: PetManifest | undefined, state: string): Animation | undefined {
    return manifest && Object.prototype.hasOwnProperty.call(manifest.animations, state) ? manifest.animations[state] : undefined;
  }

  private atlas(url: string): Promise<HTMLImageElement> {
    const cached = this.atlases.get(url);
    if (cached) return cached;
    const image = new Image();
    image.src = url;
    const decoded = image.decode().then(() => image).catch((error: unknown) => {
      if (this.atlases.get(url) === decoded) this.atlases.delete(url);
      throw error;
    });
    this.atlases.set(url, decoded);
    return decoded;
  }

  private cancelFrame(): void {
    if (this.frame !== undefined) cancelAnimationFrame(this.frame);
    this.frame = undefined;
  }

  private scheduleFrame(): void {
    if (!this.visible || !this.mounted?.image || this.frame !== undefined) return;
    const frame = requestAnimationFrame((timestamp) => {
      if (this.frame !== frame) return;
      this.frame = undefined;
      this.tick(timestamp);
    });
    this.frame = frame;
  }

  private tick(timestamp: number): void {
    const mount = this.mounted;
    if (!this.visible || !mount?.image) return;

    const staticFrame = this.options.staticFrame;
    if (staticFrame || this.cadence.shouldDraw(timestamp)) {
      const state = this.state;
      const sample = this.clock.sample(mount.manifest.animations[state], staticFrame ? 0 : timestamp - this.animationStartedAt);
      const canvas = mount.canvas;
      const rect = sample.rect;
      const scale = Math.min(canvas.width / rect.width, canvas.height / rect.height);
      const drawWidth = rect.width * scale;
      const drawHeight = rect.height * scale;
      mount.context.clearRect(0, 0, canvas.width, canvas.height);
      mount.context.drawImage(mount.image, rect.x, rect.y, rect.width, rect.height,
        (canvas.width - drawWidth) / 2, (canvas.height - drawHeight) / 2, drawWidth, drawHeight);
      if (this.options.onPose) {
        const mask = mount.masks.sample(rect, canvas.width, canvas.height, () =>
          mount.context.getImageData(0, 0, canvas.width, canvas.height).data);
        if (mask) this.options.onPose(mask, rect);
      }
      this.options.onDraw?.(performance.now(), state, sample);
    }

    if (!staticFrame) this.scheduleFrame();
  }
}
