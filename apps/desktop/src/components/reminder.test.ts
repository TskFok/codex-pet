import { expect, it } from 'vitest';
import fixture from '../../../../tests/fixtures/events/child-folding.json';
import { presentation, withBubble, withBubbles } from './reminder';
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
    expect(result.reminderId).toBeUndefined();
  }
});

it('同一提醒在无关快照变化下保持稳定标识，新请求或新通知产生不同标识', () => {
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  const first = presentation(snap, []);
  expect(first.reminderId).toBeTruthy();
  const unrelated = structuredClone(snap);
  unrelated.revision += 1;
  unrelated.sessions.push({
    key: { source: 'deepseek', installationId: 'other', sessionId: 'bg' },
    status: 'running',
    pending: [],
    unread: false,
    lastEventAtMs: 999,
  });
  expect(presentation(unrelated, []).reminderId).toBe(first.reminderId);
  const nextReq = structuredClone(snap);
  nextReq.sessions[0].pending = [{ id: 'req-2', reason: 'answer', summary: '新的问题' }];
  expect(presentation(nextReq, []).reminderId).not.toBe(first.reminderId);
});

it('任务结束与失败给出稳定动作键与回退循环状态，失败标为已读后动作键不变', () => {
  const key = { source: 'codex' as const, installationId: 'i1', sessionId: 's1' };
  const endedSnap: PetSnapshot = {
    revision: 1,
    sessions: [
      { key, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 },
    ],
    notifications: [
      {
        id: 'n1',
        key,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        occurredAtMs: 100,
        read: false,
      },
    ],
  };
  const idleResult = presentation(endedSnap, []);
  expect(idleResult.state).toBe('waving');
  expect(idleResult.fallbackState).toBe('idle');
  expect(idleResult.animationKey).toBeTruthy();

  const withRunning: PetSnapshot = {
    ...endedSnap,
    sessions: [
      ...endedSnap.sessions,
      {
        key: { ...key, sessionId: 's2' },
        turnId: 't2',
        status: 'running',
        pending: [],
        unread: false,
        lastEventAtMs: 110,
      },
    ],
  };
  expect(presentation(withRunning, []).fallbackState).toBe('running');

  const failedUnread: PetSnapshot = {
    revision: 2,
    sessions: [
      { key, turnId: 't1', status: 'failed', pending: [], unread: true, lastEventAtMs: 200 },
    ],
    notifications: [
      {
        id: 'n2',
        key,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'failed' },
        occurredAtMs: 200,
        read: false,
      },
    ],
  };
  const failedRead: PetSnapshot = {
    ...failedUnread,
    revision: 3,
    sessions: [{ ...failedUnread.sessions[0], unread: false }],
    notifications: [{ ...failedUnread.notifications[0], read: true }],
  };
  expect(presentation(failedUnread, []).state).toBe('failed');
  expect(presentation(failedUnread, []).animationKey).toBe(presentation(failedRead, []).animationKey);
});

it('多个会话同时存在未读提醒时全部返回，并按优先级与时间排序；多气泡掩码保留间隙穿透', () => {
  const keyA = { source: 'codex' as const, installationId: 'i1', sessionId: 's-ended' };
  const keyB = { source: 'deepseek' as const, installationId: 'i2', sessionId: 's-attention' };
  const keyC = { source: 'antigravity' as const, installationId: 'i3', sessionId: 's-ended-2' };
  const snap: PetSnapshot = {
    revision: 5,
    sessions: [
      { key: keyA, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 },
      {
        key: keyB,
        turnId: 't2',
        status: 'needs_attention',
        pending: [{ id: 'p1', reason: 'approval', summary: '需要确认命令' }],
        unread: true,
        lastEventAtMs: 150,
      },
      { key: keyC, turnId: 't3', status: 'ended', pending: [], unread: true, lastEventAtMs: 200 },
    ],
    notifications: [
      {
        id: 'n1',
        key: keyA,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        occurredAtMs: 100,
        read: false,
      },
      {
        id: 'n3',
        key: keyC,
        turnId: 't3',
        kind: { type: 'turn_ended', outcome: 'completed' },
        occurredAtMs: 200,
        read: false,
      },
    ],
  };

  const result = presentation(snap, []);
  expect(result.reminders).toHaveLength(3);
  expect(result.reminders.map((r) => r.key.sessionId)).toEqual(['s-attention', 's-ended-2', 's-ended']);
  expect(result.title).toBe(result.reminders[0].title);

  // 静音其中一个来源只过滤该来源，其余会话提醒仍保留
  const mutedResult = presentation(snap, ['deepseek']);
  expect(mutedResult.reminders.map((r) => r.key.sessionId)).toEqual(['s-ended-2', 's-ended']);

  // 多气泡合并只覆盖各气泡矩形，不填充气泡之间的透明间隙
  expect(
    withBubbles(
      { width: 8, height: 5, runs: [[4, 2, 6]] },
      [
        { x: 1, y: 0, width: 6, height: 1 },
        { x: 1, y: 2, width: 6, height: 1 },
      ],
    ).runs,
  ).toEqual([
    [0, 1, 7],
    [2, 1, 7],
    [4, 2, 6],
  ]);
});

it('关闭 subagent 提醒时同时过滤 isSubagent 与 parentSessionId 子代理的动画与气泡，保留主会话', () => {
  const mainKey = { source: 'codex' as const, installationId: 'i1', sessionId: 'root-session' };
  const codexSubKey = { source: 'codex' as const, installationId: 'i1', sessionId: 'child-thread' };
  const dshSubKey = { source: 'deepseek' as const, installationId: 'i2', sessionId: 'dsh-child' };
  const snap: PetSnapshot = {
    revision: 8,
    sessions: [
      {
        key: mainKey,
        turnId: 't-main',
        status: 'running',
        pending: [],
        unread: false,
        lastEventAtMs: 100,
      },
      {
        key: codexSubKey,
        isSubagent: true,
        turnId: 't-sub-1',
        status: 'unknown',
        pending: [],
        unread: true,
        lastEventAtMs: 200,
      },
      {
        key: dshSubKey,
        parentSessionId: 'dsh-root',
        turnId: 't-sub-2',
        status: 'needs_attention',
        pending: [{ id: 'p-sub', reason: 'approval', summary: '子代理审批' }],
        unread: true,
        lastEventAtMs: 300,
      },
    ],
    notifications: [
      {
        id: 'n-sub-end',
        key: codexSubKey,
        turnId: 't-sub-1',
        kind: { type: 'turn_ended', outcome: 'unknown' },
        occurredAtMs: 200,
        read: false,
      },
    ],
  };

  // 默认开启时同时展示两个子代理提醒，并触发审批动画
  const shown = presentation(snap, [], true);
  expect(shown.state).toBe('review');
  expect(shown.reminders.map((r) => r.key.sessionId)).toEqual(['dsh-child', 'child-thread']);

  // 关闭 subagent 提醒后，不再弹出气泡，动画回退到主会话的 running 状态
  const hidden = presentation(snap, [], false);
  expect(hidden.state).toBe('running');
  expect(hidden.fallbackState).toBe('running');
  expect(hidden.reminders).toHaveLength(0);
  expect(hidden.title).toBeUndefined();
  expect(hidden.detail).toBeUndefined();

  // 若仅有 subagent 结束且主会话已空闲，关闭后保持 idle 且无提醒
  const onlySubagent: PetSnapshot = {
    revision: 9,
    sessions: [snap.sessions[1]],
    notifications: snap.notifications,
  };
  const idle = presentation(onlySubagent, [], false);
  expect(idle.state).toBe('idle');
  expect(idle.fallbackState).toBe('idle');
  expect(idle.reminders).toHaveLength(0);
});
