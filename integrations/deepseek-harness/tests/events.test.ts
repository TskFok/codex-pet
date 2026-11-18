import { describe, expect, it } from 'vitest';
import { toPetEvents, type HarnessObservation } from '../src/events.js';
const identity = { source: 'deepseek' as const, installationId: 'install-1', instanceId: 'session-log:child:100' };
const base = { sessionId: 'child', sessionCreatedAt:100, parentSessionId: 'parent', turn: 1, seq: 10, time: 123 };
describe('宿主事件归一化', () => {
  it('approval_decided_clears_only_matching_request', () => {
    const events = toPetEvents({ ...base, type: 'approval/decided', requestId: 'approval-a' }, identity);
    expect(events).toHaveLength(1);
    expect(events[0]?.kind).toEqual({ type: 'attention_resolved', requestId: 'approval-a' });
    expect(events[0]?.sequence).toBe(10);
  });
  it('parent_child_ids_remain_distinct', () => {
    const event = toPetEvents({ ...base, type: 'turn/start' }, identity)[0];
    expect(event?.sessionId).toBe('child');
    expect(event?.parentSessionId).toBe('parent');
    expect(event?.turnId).toBe('1');
  });
  it('取消提问只解除指定请求，使用实际源序号', () => {
    const observed: HarnessObservation = { ...base, type: 'question/resolved', requestId: 'question:q1' };
    expect(toPetEvents(observed, identity)[0]?.kind).toEqual({ type: 'attention_resolved', requestId: 'question:q1' });
  });
  it('序号、事件标识在重放时稳定，摘要不携带工具参数', () => {
    const event = toPetEvents({ ...base, type: 'approval/asked', requestId: 'a' }, identity)[0];
    expect(event?.eventId).toBe(toPetEvents({ ...base, type: 'approval/asked', requestId: 'a' }, identity)[0]?.eventId);
    expect(event?.upstreamEventId).toBe('child:10');
    expect(event?.summary).toBeUndefined();
  });
});
