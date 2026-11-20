import { expect, it, vi } from 'vitest';
import { PoseMasks } from './hit-mask';

it('逐姿态缓存真实绘制 alpha，保留半透明与空洞，尺寸变化后重新采样', () => {
  const cache = new PoseMasks();
  const pixels = new Uint8ClampedArray(8 * 4);
  [0, 1, 255, 0, 2, 0, 0, 255].forEach((alpha, i) => pixels[i * 4 + 3] = alpha);
  const read = vi.fn(() => pixels);
  const a = { x: 0, y: 0, width: 2, height: 2 };
  const b = { ...a, x: 2 };
  const mask = cache.sample(a, 4, 2, read);
  expect(mask).toEqual({ width: 4, height: 2, runs: [[0, 1, 3], [1, 0, 1], [1, 3, 4]] });
  expect(cache.sample(a, 4, 2, read)).toBeUndefined();
  cache.sample(b, 4, 2, read);
  expect(cache.sample(a, 4, 2, read)).toBe(mask);
  cache.sample(a, 2, 4, read);
  expect(read).toHaveBeenCalledTimes(3);
});

it('不同动作引用同一个图集矩形时沿用已经安装的命中区域', () => {
  const cache = new PoseMasks();
  const frame = { x: 12, y: 0, width: 2, height: 1 };
  const read = vi.fn(() => new Uint8ClampedArray(8));
  cache.sample(frame, 2, 1, read);
  expect(cache.sample({ ...frame }, 2, 1, read)).toBeUndefined();
  expect(read).toHaveBeenCalledTimes(1);
});
