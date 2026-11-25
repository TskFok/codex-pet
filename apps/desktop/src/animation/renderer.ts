import { FrameCadence } from '../diagnostics/frame-cadence';
import type { Animation, Atlas, FrameRect, PetManifest } from '../types';
import { AnimationClock, type AnimationSample } from './clock';
import { PetEffects, type InteractionEffect } from './effects';
import { PoseMasks, type AlphaMask } from './hit-mask';

interface RendererOptions {
  onPose?: (mask: AlphaMask, frame: FrameRect) => void;
  resolveAtlasUrl: (atlas: Atlas) => string;
  viewport?: (width: number, height: number) => FrameRect;
  staticFrame?: boolean;
  onDraw?: (timestamp: number, state: string, sample: AnimationSample) => void;
  onAnimationEnd?: (state: string, timestamp: number) => void;
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
  private readonly effects = new PetEffects();
  private mounted?: Mount;
  private visible = false;
  private state = 'idle';
  private animationStartedAt = 0;
  private animationEnded = false;
  private frame?: number;
  private fps: 30 | 60 = 30;
  private cadence = new FrameCadence();

  constructor(private readonly options: RendererOptions) {}

  mount(canvas: HTMLCanvasElement, manifest: PetManifest, atlasUrl?: string): Promise<void> {
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
    this.animationEnded = false;
    this.cadence = new FrameCadence(this.fps);
    const ready = this.atlas(atlasUrl ?? this.options.resolveAtlasUrl(manifest.atlas)).then((image) => {
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

  hasAnimation(state: string): boolean {
    return this.animation(this.mounted?.manifest, state) !== undefined;
  }

  triggerInteractionEffect(effect: InteractionEffect, nowMs = performance.now()): void {
    if (this.options.staticFrame) return;
    this.effects.trigger(effect, nowMs);
  }

  setState(state: string, nowMs: number, restart = false): boolean {
    if (!this.animation(this.mounted?.manifest, state)) return false;
    if (state === this.state && !restart) return true;
    this.state = state;
    this.animationStartedAt = nowMs;
    this.animationEnded = false;
    if (this.options.staticFrame) this.scheduleFrame();
    return true;
  }

  setVisible(visible: boolean, nowMs: number): void {
    if (this.visible === visible) return;
    this.visible = visible;
    if (!visible) {
      this.cancelFrame();
      this.effects.clear();
      return;
    }
    this.animationStartedAt = nowMs;
    this.animationEnded = false;
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
    this.effects.clear();
    this.mounted = undefined;
    this.visible = false;
    this.state = 'idle';
    this.animationEnded = false;
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
    // 私有图集的 data URL 可能较大。缓存仅保留当前和上一个图集。
    if (this.atlases.size >= 2) this.atlases.delete(this.atlases.keys().next().value!);
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
      let state = this.state;
      let sample = this.clock.sample(
        mount.manifest.animations[state],
        staticFrame ? 0 : timestamp - this.animationStartedAt,
      );
      if (!staticFrame && sample.finished && !this.animationEnded) {
        this.animationEnded = true;
        this.options.onAnimationEnd?.(state, timestamp);
        if (!this.visible || this.mounted !== mount || !mount.image) return;
        if (this.state !== state) {
          state = this.state;
          sample = this.clock.sample(
            mount.manifest.animations[state],
            timestamp - this.animationStartedAt,
          );
        }
      }
      const canvas = mount.canvas;
      const rect = sample.rect;
      const viewport = this.options.viewport?.(canvas.width, canvas.height) ?? {
        x: 0,
        y: 0,
        width: canvas.width,
        height: canvas.height,
      };
      const scale = Math.min(viewport.width / rect.width, viewport.height / rect.height);
      const drawWidth = rect.width * scale;
      const drawHeight = rect.height * scale;
      const drawX = viewport.x + (viewport.width - drawWidth) / 2;
      const drawY = viewport.y + (viewport.height - drawHeight) / 2;
      const bounce = staticFrame ? undefined : this.effects.bounceAt(timestamp);
      const drawSprite = (withBounce: boolean) => {
        mount.context.clearRect(0, 0, canvas.width, canvas.height);
        if (withBounce && bounce) {
          const bw = drawWidth * bounce.scaleX;
          const bh = drawHeight * bounce.scaleY;
          const bx = drawX + (drawWidth - bw) / 2;
          const by = drawY + (drawHeight - bh) + bounce.offsetY * drawHeight;
          mount.context.drawImage(mount.image!, rect.x, rect.y, rect.width, rect.height, bx, by, bw, bh);
        } else {
          mount.context.drawImage(
            mount.image!,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            drawX,
            drawY,
            drawWidth,
            drawHeight,
          );
        }
      };
      if (this.options.onPose) {
        const left = Math.max(0, Math.floor(drawX));
        const top = Math.max(0, Math.floor(drawY));
        const region = {
          x: left,
          y: top,
          width: Math.max(0, Math.min(canvas.width, Math.ceil(drawX + drawWidth)) - left),
          height: Math.max(0, Math.min(canvas.height, Math.ceil(drawY + drawHeight)) - top),
        };
        if (bounce && mount.masks.hasCached(rect, canvas.width, canvas.height, region)) {
          const mask = mount.masks.sample(
            rect,
            canvas.width,
            canvas.height,
            () => mount.context.getImageData(region.x, region.y, region.width, region.height).data,
            region,
          );
          if (mask) this.options.onPose(mask, rect);
          drawSprite(true);
        } else {
          drawSprite(false);
          const mask = mount.masks.sample(
            rect,
            canvas.width,
            canvas.height,
            () => mount.context.getImageData(region.x, region.y, region.width, region.height).data,
            region,
          );
          if (mask) this.options.onPose(mask, rect);
          if (bounce) drawSprite(true);
        }
      } else {
        drawSprite(true);
      }
      if (!staticFrame) {
        this.effects.drawParticles(mount.context, canvas.width, canvas.height, timestamp);
      }
      this.options.onDraw?.(performance.now(), state, sample);
    }

    if (!staticFrame) this.scheduleFrame();
  }
}
