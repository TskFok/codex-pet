import { expect, it } from 'vitest';
import { serveSessionLinks, type SessionLinkBridge } from '../src/session-links-client.js';
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
