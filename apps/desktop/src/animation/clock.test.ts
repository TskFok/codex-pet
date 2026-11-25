import { expect, it } from 'vitest';
import type { Animation, AnimationFrame } from '../types';
import { AnimationClock } from './clock';

const frames: AnimationFrame[] = [
  { rect: { x: 0, y: 0, width: 10, height: 10 }, durationMs: 100 },
  { rect: { x: 10, y: 0, width: 10, height: 10 }, durationMs: 250 },
  { rect: { x: 0, y: 10, width: 10, height: 10 }, durationMs: 75 },
];
const clock = new AnimationClock();

it('按不等长的持续时间在准确边界切换到下一帧', () => {
  const animation: Animation = { frames };
  expect([0, 99, 100, 349, 350].map((elapsed) => clock.sample(animation, elapsed).frameIndex))
    .toEqual([0, 0, 1, 1, 2]);
  expect(clock.sample(animation, 350).rect).toEqual({ x: 0, y: 10, width: 10, height: 10 });
});

it('只播放一次开场，随后从 loopStart 循环并能直接跳过 5 秒', () => {
  const animation: Animation = { frames, loopStart: 1 };
  expect([424, 425, 674, 675, 750, 5000].map((elapsed) => clock.sample(animation, elapsed).frameIndex))
    .toEqual([2, 1, 1, 2, 1, 1]);
  expect(clock.sample(animation, 5000).finished).toBe(false);
});

it('单次动作结束后一直保持末帧并标记已结束', () => {
  const animation: Animation = { frames };
  expect(clock.sample(animation, 424)).toMatchObject({ frameIndex: 2, finished: false });
  expect(clock.sample(animation, 425)).toMatchObject({ frameIndex: 2, finished: true });
  expect(clock.sample(animation, 5000)).toMatchObject({ frameIndex: 2, finished: true });
});
