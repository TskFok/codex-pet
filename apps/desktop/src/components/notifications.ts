import { api, message, observeSnapshot } from '../api';
import { historyName, reasonName, sourceName, statusName } from '../locales/zh-CN';
import type { NotificationRecord, PetSnapshot, SessionKey, SessionProjection } from '../types';

export const keyId = (key: SessionKey) => JSON.stringify([key.source, key.installationId, key.sessionId]);
type Actions = { markRead: (key: SessionKey) => void; clear: () => void };
export function renderNotifications(root: HTMLElement, snapshot: PetSnapshot, actions: Actions): void {
  const doc = root.ownerDocument;
  const opened = new Set(
    Array.from(root.querySelectorAll<HTMLDetailsElement>('details[open]'), (d) => d.dataset.key),
  );
  const el = <K extends keyof HTMLElementTagNameMap>(tag: K, text = '') => {
    const node = doc.createElement(tag);
    node.textContent = text;
    return node;
  };
  const button = (text: string, click: () => void) => {
    const b = el('button', text);
    b.onclick = click;
    return b;
  };
  const groups = new Map<string, NotificationRecord[]>();
  for (const n of snapshot.notifications) {
    const id = keyId(n.key);
    if (!groups.has(id)) groups.set(id, []);
    groups.get(id)!.push(n);
  }
  const sessions = new Map(snapshot.sessions.map((s) => [keyId(s.key), s]));
  const byParent = new Map<string, SessionProjection[]>();
  for (const s of sessions.values()) {
    if (!s.parentSessionId) continue;
    const parent = keyId({ ...s.key, sessionId: s.parentSessionId });
    if (!sessions.has(parent)) continue;
    if (!byParent.has(parent)) byParent.set(parent, []);
    byParent.get(parent)!.push(s);
  }
  const seen = new Set<string>();
  function card(key: SessionKey, s?: SessionProjection): HTMLDetailsElement {
    const id = keyId(key);
    seen.add(id);
    const rows = groups.get(id) || [];
    const newest = rows[rows.length - 1];
    const d = el('details');
    d.className = 'session';
    d.dataset.key = id;
    d.open = opened.has(id);
    const summary = el('summary');
    const unread = s?.unread || rows.some((n) => !n.read);
    summary.append(
      el('strong', s?.project?.name || newest?.project?.name || key.sessionId),
      el('span', sourceName[key.source]),
    );
    if (unread) {
      const badge = el('small', '未读');
      badge.className = 'unread';
      summary.append(badge);
    }
    if (s) summary.append(el('small', statusName(s)));
    d.append(summary, el('p', '会话 ' + key.sessionId));
    if (s?.summary) d.append(el('p', s.summary));
    for (const p of s?.pending || [])
      d.append(
        el(
          'p',
          [
            s?.status === 'needs_attention' && s.pending.length === 1
              ? reasonName[p.reason]
              : p.reason === 'approval'
                ? '审批请求记录'
                : '提问记录',
            p.summary,
          ]
            .filter(Boolean)
            .join(' · '),
        ),
      );
    if (unread) d.append(button('标为已读', () => actions.markRead(key)));
    for (const n of rows.slice().reverse()) {
      const text = [historyName(n.kind), n.summary, new Date(n.occurredAtMs).toLocaleString('zh-CN')]
        .filter(Boolean)
        .join(' · ');
      const item = el('p', text);
      item.className = 'history';
      d.append(item);
    }
    // 深层关系按需查看，孤立或循环的源关系仍只呈现每个会话一次。
    for (const child of byParent.get(id) || [])
      if (!seen.has(keyId(child.key))) d.append(card(child.key, child));
    return d;
  }
  const header = el('header');
  header.append(el('h1', '通知'));
  if (snapshot.notifications.length) header.append(button('清空历史', actions.clear));
  const fragment = doc.createDocumentFragment();
  fragment.append(header);
  const sorted = [...sessions.values()].sort((a, b) => b.lastEventAtMs - a.lastEventAtMs);
  const roots = sorted.filter(
    (s) => !s.parentSessionId || !sessions.has(keyId({ ...s.key, sessionId: s.parentSessionId })),
  );
  for (const s of [...roots, ...sorted]) if (!seen.has(keyId(s.key))) fragment.append(card(s.key, s));
  groups.forEach((rows, id) => {
    if (!seen.has(id)) fragment.append(card(rows[0].key));
  });
  if (!seen.size) fragment.append(el('p', '暂无通知'));
  root.replaceChildren(fragment);
}
export async function mountNotifications(root: HTMLElement): Promise<() => void> {
  const content = root.ownerDocument.createElement('section');
  const error = root.ownerDocument.createElement('p');
  error.setAttribute('role', 'alert');
  root.replaceChildren(content, error);
  let latest = -1;
  const render = (s: PetSnapshot) => {
    if (s.revision < latest) return;
    latest = s.revision;
    error.textContent = '';
    renderNotifications(content, s, {
      markRead: (key) => run(api.markRead(key)),
      clear: () => {
        if (window.confirm('清空通知历史？仍待处理的请求会保留。')) run(api.clear());
      },
    });
  };
  const run = (work: Promise<PetSnapshot>) => {
    void work.then(render).catch(showError);
  };
  function showError(e: unknown) {
    error.textContent = message(e) + ' ';
    const retry = root.ownerDocument.createElement('button');
    retry.textContent = '重试';
    retry.onclick = () => run(api.snapshot());
    error.append(retry);
  }
  return observeSnapshot(api.snapshots, api.snapshot, render, showError);
}
