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

it('已确认的上回合未读结束在下一回合运行时仍显示，处理后消失', () => {
  const key = { source: 'deepseek' as const, installationId: 'fixture', sessionId: 'original' };
  const s = {
    key,
    status: 'running' as const,
    summary: '新回合摘要',
    unread: true,
    lastEventAtMs: 1,
    pending: [],
  };
  const snap = {
    revision: 2,
    sessions: [s, { ...s, key: { ...key, sessionId: 'newer' }, lastEventAtMs: 2 }],
    notifications: [],
    importedUnreadEnds: [key],
  };
  expect(presentation(snap, []).detail).toContain('结束');
  expect(presentation(snap, []).title).toContain('original');
  expect(presentation(snap, []).detail).not.toContain('新回合摘要');
  expect(presentation({ ...snap, importedUnreadEnds: [] }, []).detail).toBeUndefined();
  expect(presentation(snap, ['deepseek']).detail).toBeUndefined();
});

it('更高优先级来源被静音时，保留其动作，气泡展示另一个来源已确认的上一回合结束', () => {
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  const key = { source: 'deepseek' as const, installationId: 'other', sessionId: 'prior-turn' };
  snap.sessions.push({ key, status: 'running', unread: true, pending: [], lastEventAtMs: 200 });
  snap.importedUnreadEnds = [key];
  const result = presentation(snap, ['codex']);
  expect(result.state).toBe('review');
  expect(result.title).toContain('prior-turn');
  expect(result.detail).toContain('结束');
});
it('来源设置未知或静音时隐藏提醒内容，保持已确认的审批动画', () => {
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  for (const muted of [undefined, ['codex'] as const]) {
    const result = presentation(snap, muted ? [...muted] : undefined);
    expect(result.state).toBe('review');
    expect(result.title).toBeUndefined();
    expect(result.detail).toBeUndefined();
  }
});
