import { expect, it } from 'vitest';
import {
  consumeWebSessionLink,
  parseWebSessionHash,
  serveSessionLinks,
  type SessionLinkBridge,
  type WebSessionLocationWindow,
} from '../src/session-links-client.js';
it('会话存在才定位，未就绪保留目标，错误确认不可用，释放引用', async () => {
  let change = () => {};
  const queue = [{ id: 1, sessionId: 'exists' }, { id: 2, sessionId: 'missing' }];
  const acks: [number, string][] = [];
  const opened: string[] = [];
  const released: string[] = [];
  const bridge: SessionLinkBridge = {
    pending: async () => [...queue],
    subscribe: listener => { change = listener; return () => { change = () => {}; }; },
    acknowledge: async (id, status) => { acks.push([id, status]); queue.splice(queue.findIndex(row => row.id === id), 1); },
  };
  const lifetime = new AbortController();
  const stop = serveSessionLinks(bridge, async (id) => {
    try { if (id === 'missing') throw Error('not found'); opened.push(id); }
    finally { released.push(id); }
  }, lifetime.signal, () => {});
  await stop.settled();
  expect(opened).toEqual(['exists']);
  expect(acks).toEqual([[1, 'opened'], [2, 'unavailable']]);
  expect(released).toEqual(['exists', 'missing']);
  stop.dispose();
  change();
  expect(queue).toEqual([]);
});
it('插件撤销时未完成的打开不确认，不在撤销后切换会话', async () => {
  const lifetime = new AbortController();
  const gate = Promise.withResolvers<void>();
  let ack = false;
  const bridge: SessionLinkBridge = { pending: async () => [{id:1, sessionId:'slow'}], subscribe: () => () => {}, acknowledge: async () => {ack=true;} };
  const consumer = serveSessionLinks(bridge, () => gate.promise, lifetime.signal, () => {});
  await Promise.resolve(); await Promise.resolve();
  consumer.dispose(); gate.resolve(); await consumer.settled();
  expect(ack).toBe(false);
});
it('Harness Web 哈希入口解析受限会话标识并拒绝非法输入', () => {
  expect(parseWebSessionHash('#codex-pet-session=session-1')).toBe('session-1');
  expect(parseWebSessionHash('#codex-pet-session=%E4%BC%9A%E8%AF%9D%2Fa%20%3F%23')).toBe('会话/a ?#');
  for (const invalid of [
    '',
    '#other=1',
    '#codex-pet-session=',
    '#codex-pet-session=%ZZ',
    '#codex-pet-session=%20',
    '#codex-pet-session=..',
    '#codex-pet-session=https%3A%2F%2Fevil.example',
    '#codex-pet-session=x%00y',
  ]) {
    expect(parseWebSessionHash(invalid)).toBeUndefined();
  }
});
it('Harness Web 消费初始与后续哈希目标并清理 URL，失败时提示', async () => {
  let listener: (() => void) | undefined;
  const location = { hash: '#codex-pet-session=session-1', pathname: '/', search: '?keep=1' };
  const replaced: (string | URL | null | undefined)[] = [];
  const win: WebSessionLocationWindow = {
    location,
    history: { replaceState: (_data, _unused, url) => { replaced.push(url); location.hash = ''; } },
    addEventListener: (_type, cb) => { listener = cb; },
    removeEventListener: (_type, cb) => { if (listener === cb) listener = undefined; },
  };
  const opened: string[] = [];
  let warnings = 0;
  const lifetime = new AbortController();
  const consumer = consumeWebSessionLink(win, async (id) => {
    if (id === 'missing') throw Error('not found');
    opened.push(id);
  }, lifetime.signal, () => { warnings += 1; });
  await consumer.settled();
  expect(opened).toEqual(['session-1']);
  expect(replaced).toEqual(['/?keep=1']);
  expect(warnings).toBe(0);
  location.hash = '#codex-pet-session=missing';
  listener?.();
  await consumer.settled();
  expect(warnings).toBe(1);
  consumer.dispose();
  expect(listener).toBeUndefined();
});
