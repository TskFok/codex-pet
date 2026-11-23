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

it('从历史尝试导航，保留会话，并仅在明确点击时降级打开应用', async () => {
  const { document } = parseHTML('<html><body></body></html>');
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  const key = snap.sessions[0].key;
  const open = vi.fn()
    .mockResolvedValueOnce({ status: 'application_only', opened: false })
    .mockResolvedValueOnce({ status: 'application_only', opened: true })
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
  button('尝试打开会话').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('可以打开应用'));
  expect(open).toHaveBeenCalledTimes(1);
  expect(open).toHaveBeenLastCalledWith(key, 'session');
  button('打开应用').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('已发送打开应用请求'));
  expect(open).toHaveBeenLastCalledWith(key, 'application');
  button('尝试打开会话').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('已发送打开请求'));
  button('尝试打开会话').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('系统未接受打开请求'));
  button('复制会话 ID').click();
  await vi.waitFor(() => expect(entry.textContent).toContain('已复制会话 ID'));
  expect(copy).toHaveBeenCalledWith(key);
  expect(entry.querySelector('.history')).toBeTruthy();
});
