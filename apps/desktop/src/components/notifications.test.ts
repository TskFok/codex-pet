import { parseHTML } from 'linkedom';
import { expect, it, vi } from 'vitest';
import fixture from '../../../../tests/fixtures/events/child-folding.json';
import { renderNotifications } from './notifications';
import type { PetSnapshot } from '../types';

it('由快照更新项目、摘要、未读和待处理，同时保留可查看的会话', () => {
  const { document } = parseHTML('<html><body></body></html>');
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  const row = snap.sessions[0];
  row.project = { name: '花园' };
  row.pending[0].summary = '核对更改';
  const actions = { markRead: vi.fn(), clear: vi.fn(), open: vi.fn(), copy: vi.fn() };
  renderNotifications(document.body, snap, actions);
  for (const label of ['Codex', '花园', '核对更改', '待审批', '未读'])
    expect(document.body.textContent).toContain(label);
  expect(document.body.querySelector('details')?.textContent).toContain('child');
  row.pending = [];
  row.status = 'running';
  row.unread = false;
  for (const n of snap.notifications) {
    n.read = true;
    if (n.kind.type === 'attention') n.kind.resolved = true;
  }
  renderNotifications(document.body, snap, actions);
  expect(document.body.textContent).not.toContain('待审批');
  const child = document.body.querySelector('details details')!;
  expect(child.textContent).not.toContain('未读');
  expect(document.body.textContent).toContain('未读');
});

it('从历史可直接点击打开应用，尝试打开会话不会隐式降级，并保留会话与复制操作', async () => {
  const { document } = parseHTML('<html><body></body></html>');
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  const key = snap.sessions[0].key;
  const open = vi.fn()
    .mockResolvedValueOnce({ status: 'application_only', opened: true })
    .mockResolvedValueOnce({ status: 'application_only', opened: false })
    .mockResolvedValueOnce({ status: 'requested' })
    .mockRejectedValueOnce(new Error('系统未接受打开请求'));
  const copy = vi.fn().mockResolvedValue(undefined);
  renderNotifications(document.body, snap, { markRead: vi.fn(), clear: vi.fn(), open, copy });
  const entry = Array.from(document.body.querySelectorAll('details')).find(
    row => row.dataset.key === JSON.stringify([key.source, key.installationId, key.sessionId]),
  )!;
  const button = (label: string) => Array.from(entry.querySelectorAll('button')).find(
    item => item.closest('details') === entry && item.textContent === label,
  )!;
  expect(button('打开应用').hidden).toBe(false);
  button('打开应用').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('已发送打开应用请求'));
  expect(open).toHaveBeenCalledTimes(1);
  expect(open).toHaveBeenLastCalledWith(key, 'application');
  button('尝试打开会话').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('可以打开应用'));
  expect(open).toHaveBeenLastCalledWith(key, 'session');
  button('尝试打开会话').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('已发送打开请求'));
  button('尝试打开会话').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('系统未接受打开请求'));
  button('复制会话 ID').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('已复制会话 ID'));
  expect(copy).toHaveBeenCalledWith(key);
  expect(entry.querySelector('.history')).toBeTruthy();
});

it('通知面板为带有 isSubagent 或 parentSessionId 的子代理展示子代理标记', () => {
  const { document } = parseHTML('<html><body></body></html>');
  const snap: PetSnapshot = {
    revision: 1,
    sessions: [
      {
        key: { source: 'codex', installationId: 'i1', sessionId: 'codex-sub' },
        isSubagent: true,
        status: 'ended',
        pending: [],
        unread: true,
        lastEventAtMs: 200,
      },
      {
        key: { source: 'codex', installationId: 'i1', sessionId: 'codex-root' },
        status: 'ended',
        pending: [],
        unread: false,
        lastEventAtMs: 100,
      },
    ],
    notifications: [],
  };
  renderNotifications(document.body, snap, { markRead: vi.fn(), clear: vi.fn(), open: vi.fn(), copy: vi.fn() });
  const cards = Array.from(document.body.querySelectorAll('details.session'));
  expect(cards[0].querySelector('.subagent')?.textContent).toBe('子代理');
  expect(cards[1].querySelector('.subagent')).toBeNull();
});
