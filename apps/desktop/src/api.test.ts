import { expect, it } from 'vitest';
import { observeSnapshot } from './api';
import type { PetSnapshot } from './types';

it('订阅期间到达的新版覆盖较旧的首次查询，并可停止订阅', async () => {
  const snapshot = (revision: number): PetSnapshot => ({ revision, notifications: [], sessions: [] });
  const rendered: number[] = [];
  let closed = false;
  const stop = await observeSnapshot(
    async (cb) => {
      cb(snapshot(8));
      return () => {
        closed = true;
      };
    },
    async () => snapshot(7),
    (value) => rendered.push(value.revision),
  );
  expect(rendered).toEqual([8]);
  stop();
  expect(closed).toBe(true);
});
