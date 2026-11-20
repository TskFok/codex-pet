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

  sample(frame: FrameRect, width: number, height: number, read: () => Uint8ClampedArray): AlphaMask | undefined {
    if (width !== this.width || height !== this.height) {
      this.width = width;
      this.height = height;
      this.frames.clear();
      this.lastKey = undefined;
      this.last = undefined;
    }
    if (frame === this.last) return undefined;
    const key = [frame.x, frame.y, frame.width, frame.height].join(',');
    if (key === this.lastKey) { this.last = frame; return undefined; }
    const cached = this.frames.get(key);
    this.last = frame;
    this.lastKey = key;
    if (cached) return cached;
    const pixels = read();
    const runs: AlphaMask['runs'] = [];
    for (let y = 0; y < height; y++) {
      let start = -1;
      for (let x = 0; x <= width; x++) {
        const solid = x < width && pixels[(y * width + x) * 4 + 3] > 0;
        if (solid && start === -1) start = x;
        if (!solid && start >= 0) {
          runs.push([y, start, x]);
          start = -1;
        }
      }
    }
    const mask = { width, height, runs };
    this.frames.set(key, mask);
    return mask;
  }
}
