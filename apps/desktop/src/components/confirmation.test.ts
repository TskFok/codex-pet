import { parseHTML } from 'linkedom';
import { expect, it, vi } from 'vitest';
import { api } from '../api';
import type { PetSnapshot } from '../types';
import fixture from '../../../../tests/fixtures/events/child-folding.json';
import { mountNotifications } from './notifications';

it('清空通知必须先在面板明确确认，取消时保留内容', async () => {
  const { document } = parseHTML('<body><main></main></body>');
  const root = document.querySelector('main')!;
  // linkedom 提供 DOM，原生 showModal/close 由另外的 WKWebView 用例验证。
  const make = document.createElement.bind(document);
  vi.spyOn(document, 'createElement').mockImplementation((tag, options) => {
    const node = make(tag, options);
    if (tag === 'dialog')
      Object.assign(node, {
        showModal: () => node.setAttribute('open', ''),
        close: () => node.removeAttribute('open'),
      });
    return node;
  });
  const value = structuredClone(fixture.expected) as PetSnapshot;
  vi.spyOn(api, 'snapshot').mockResolvedValue(value);
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  const clear = vi.spyOn(api, 'clear').mockResolvedValue({ ...value, notifications: [], revision: 10 });
  const stop = await mountNotifications(root);
  const button = (name: string) =>
    Array.from(root.querySelectorAll('button')).find((b) => b.textContent === name)!;
  try {
    button('清空历史').click();
    expect(clear).not.toHaveBeenCalled();
    button('取消').click();
    expect(clear).not.toHaveBeenCalled();
    button('清空历史').click();
    button('确认清空').click();
    await Promise.resolve();
    expect(clear).toHaveBeenCalledTimes(1);
  } finally {
    stop();
    vi.restoreAllMocks();
  }
});
