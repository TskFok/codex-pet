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

it('按子区域采样时把非透明跨度偏移到完整画布坐标', () => {
  const cache = new PoseMasks();
  const pixels = new Uint8ClampedArray(4 * 4);
  [0, 255, 255, 0].forEach((alpha, i) => pixels[i * 4 + 3] = alpha);
  const mask = cache.sample(
    { x: 0, y: 0, width: 2, height: 2 },
    10,
    12,
    () => pixels,
    { x: 3, y: 7, width: 2, height: 2 },
  );
  expect(mask).toEqual({
    width: 10,
    height: 12,
    runs: [
      [7, 4, 5],
      [8, 3, 4],
    ],
  });
});
