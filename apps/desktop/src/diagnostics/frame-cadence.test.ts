import { expect, it } from 'vitest';
import { FrameCadence } from './frame-cadence';

it('30 FPS 在 60 Hz 回调下不因提前 0.5ms 的容差连续绘制', () => {
  const cadence = new FrameCadence();
  expect([0, 16.6, 33, 50, 66.5, 83, 100].filter((t) => cadence.shouldDraw(t)))
    .toEqual([0, 33, 66.5, 100]);
});

it('停顿后跳过过期帧，不突发补画', () => {
  const cadence = new FrameCadence();
  expect([0, 1000, 1001, 1016, 1033].filter((t) => cadence.shouldDraw(t)))
    .toEqual([0, 1000, 1033]);
});
