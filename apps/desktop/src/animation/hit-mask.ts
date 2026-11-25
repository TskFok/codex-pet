import type { FrameRect } from '../types';
export interface AlphaMask {
  width: number;
  height: number;
  /** 每个连续非透明像素段为 [行, 起点, 终点)，坐标含 Canvas 留白。 */
  runs: [number, number, number][];
}

export class PoseMasks {
  private frames = new Map<string, AlphaMask>();
  private last?: FrameRect;
  private lastKey?: string;
  private width = 0;
  private height = 0;

  hasCached(frame: FrameRect, width: number, height: number, region?: FrameRect): boolean {
    if (width !== this.width || height !== this.height) return false;
    const rx = region ? Math.max(0, Math.floor(region.x)) : 0;
    const ry = region ? Math.max(0, Math.floor(region.y)) : 0;
    const rw = region ? Math.max(0, Math.min(width - rx, Math.ceil(region.x + region.width) - rx)) : width;
    const rh = region ? Math.max(0, Math.min(height - ry, Math.ceil(region.y + region.height) - ry)) : height;
    const key = [frame.x, frame.y, frame.width, frame.height, rx, ry, rw, rh].join(',');
    return this.frames.has(key);
  }

  sample(
    frame: FrameRect,
    width: number,
    height: number,
    read: () => Uint8ClampedArray,
    region?: FrameRect,
  ): AlphaMask | undefined {
    if (width !== this.width || height !== this.height) {
      this.width = width;
      this.height = height;
      this.frames.clear();
      this.lastKey = undefined;
      this.last = undefined;
    }
    const rx = region ? Math.max(0, Math.floor(region.x)) : 0;
    const ry = region ? Math.max(0, Math.floor(region.y)) : 0;
    const rw = region ? Math.max(0, Math.min(width - rx, Math.ceil(region.x + region.width) - rx)) : width;
    const rh = region ? Math.max(0, Math.min(height - ry, Math.ceil(region.y + region.height) - ry)) : height;
    const key = [frame.x, frame.y, frame.width, frame.height, rx, ry, rw, rh].join(',');
    if (frame === this.last && key === this.lastKey) return undefined;
    if (key === this.lastKey) { this.last = frame; return undefined; }
    const cached = this.frames.get(key);
    this.last = frame;
    this.lastKey = key;
    if (cached) return cached;
    const runs: AlphaMask['runs'] = [];
    if (rw > 0 && rh > 0) {
      const pixels = read();
      for (let y = 0; y < rh; y++) {
        let start = -1;
        for (let x = 0; x <= rw; x++) {
          const solid = x < rw && pixels[(y * rw + x) * 4 + 3] > 0;
          if (solid && start === -1) start = x;
          if (!solid && start >= 0) {
            runs.push([ry + y, rx + start, rx + x]);
            start = -1;
          }
        }
      }
    }
    const mask = { width, height, runs };
    this.frames.set(key, mask);
    return mask;
  }
}
