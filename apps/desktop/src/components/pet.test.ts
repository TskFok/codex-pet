import { expect, it, vi } from 'vitest';
import { PetVisibility, type PlaybackSignal } from './pet';

it('拒绝晚到的旧可见状态，跨过休眠恢复事件也会重开动作并切断测量连续性', () => {
  const setVisible = vi.fn();
  const breakContinuity = vi.fn();
  const lifecycle = new PetVisibility(setVisible, breakContinuity);
  const signal: PlaybackSignal = { revision: 0, visible: true, suspended: false };
  lifecycle.update(signal, 0);
  lifecycle.update({ ...signal, revision: 2 }, 3000);
  lifecycle.update({ ...signal, revision: 1, suspended: true }, 3001);
  lifecycle.setDocumentVisible(false, 3002);
  lifecycle.update({ ...signal, revision: 3, visible: false }, 3003);
  lifecycle.setDocumentVisible(true, 4100);
  lifecycle.update({ ...signal, revision: 4 }, 5000);
  expect(setVisible.mock.calls).toEqual([
    [true, 0], [false, 3000], [true, 3000], [false, 3002], [true, 5000],
  ]);
  expect(breakContinuity).toHaveBeenCalledTimes(2);
});
