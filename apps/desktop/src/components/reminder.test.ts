import { expect, it } from 'vitest';
import fixture from '../../../../tests/fixtures/events/child-folding.json';
import { presentation, withBubble } from './reminder';
import type { PetSnapshot } from '../types';
it('活动请求即使已读也保留，准确解除后不再显示旧请求，静音只影响该来源', () => {
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  snap.sessions[0].unread = false;
  expect(presentation(snap, []).detail).toContain('待审批');
  expect(presentation(snap, ['codex']).detail).toBeUndefined();
  snap.sessions[0].pending = [];
  snap.sessions[0].status = 'unknown';
  expect(presentation(snap, []).detail).not.toContain('待审批');
});
it('气泡只扩展实际矩形，不填充两者之间的透明空隙', () => {
  expect(
    withBubble(
      {
        width: 8,
        height: 3,
        runs: [
          [0, 0, 2],
          [0, 4, 5],
          [2, 2, 3],
        ],
      },
      { x: 3, y: 0, width: 3, height: 2 },
    ).runs,
  ).toEqual([
    [0, 0, 2],
    [0, 3, 6],
    [1, 3, 6],
    [2, 2, 3],
  ]);
});
