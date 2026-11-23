import { message } from '../api';
import type { OpenIntent, OpenOutcome, SessionKey } from '../types';

export type NavigationActions = {
  open: (key: SessionKey, intent: OpenIntent) => Promise<OpenOutcome>;
  copy: (key: SessionKey) => Promise<void>;
};

export function createNavigation(doc: Document, key: SessionKey, actions: NavigationActions, compact = false) {
  const root = doc.createElement('div');
  const row = doc.createElement('div');
  row.className = 'actions';
  const status = doc.createElement('p');
  status.setAttribute('role', 'status');
  const open = button(compact ? '打开会话' : '尝试打开会话', 'session', () => request('session'));
  const app = button('打开应用', 'application', () => request('application'));
  const copy = button(compact ? '复制 ID' : '复制会话 ID', 'copy', async () => {
    await actions.copy(key);
    status.textContent = '已复制会话 ID';
  });
  app.hidden = true;
  row.append(open, app, copy);
  root.append(row, status);

  function button(label: string, action: string, run: () => Promise<void>) {
    const element = doc.createElement('button');
    element.type = 'button';
    element.dataset.action = action;
    element.textContent = label;
    element.onclick = () => {
      if (element.disabled) return;
      void run().catch((error) => { status.textContent = message(error); });
    };
    return element;
  }
  async function request(intent: OpenIntent) {
    open.disabled = true;
    app.disabled = true;
    try {
      const result = await actions.open(key, intent);
      if (result.status === 'requested') {
        status.textContent = '已发送打开请求';
      } else if (result.status === 'application_only') {
        app.hidden = false;
        status.textContent = result.opened
          ? '已发送打开应用请求'
          : '当前入口可以打开应用，可复制会话 ID 后查找';
      } else {
        status.textContent = '未找到可用的桌面入口，可复制会话 ID';
      }
    } finally {
      open.disabled = false;
      app.disabled = false;
    }
  }
  return root;
}
