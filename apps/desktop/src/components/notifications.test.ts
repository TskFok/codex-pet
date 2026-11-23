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
  const actions = { markRead: vi.fn(), clear: vi.fn() };
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
