import { readFileSync } from 'node:fs';
import { timedRequestId } from '../src/host-recovery.js';
import { describe, expect, it } from 'vitest';
import { harnessIdentity, toPetEvents, type HarnessObservation } from '../src/events.js';
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
    expect(event?.isSubagent).toBeUndefined();
    expect(event?.turnId).toBe('1');
  });
  it('subagent_flag_forwards_only_when_true', () => {
    const sub = toPetEvents({ ...base, isSubagent: true, type: 'turn/end', outcome: 'completed' }, identity)[0];
    expect(sub?.isSubagent).toBe(true);
    const root = toPetEvents({ ...base, parentSessionId: undefined, type: 'turn/end', outcome: 'completed' }, identity)[0];
    expect(root?.isSubagent).toBeUndefined();
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

it('超限的定时恢复范围退回整域未知，仍保留同一代的会话切点', () => {
  const observation = {
    sessionId: 's', sessionCreatedAt: 100, time: 100, type: 'session/recovered' as const, seq: 1_999,
    observation: {
      version: 1 as const, streamId: '11111111-1111-4111-8111-111111111111',
      epochId: '22222222-2222-4222-8222-222222222222', generation: 1, recoveredEventCount: 0, ancestors: [],
    },
    recoverySnapshot: {
      snapshotEventCount: 2_000, running: false,
      timed: Array.from({ length: 1_500 }, (_, birthSequence) => ({
        requestId: `durable-question-${birthSequence}`, birthSequence,
      })),
    },
  };
  const [actual] = toPetEvents(observation, harnessIdentity(observation, 'installation'));
  expect(actual).toMatchObject({
    schemaVersion: 3, sequence: 1_999, kind: { type: 'source_recovery', snapshotEventCount: 2_000 },
  });
  expect(actual?.kind).not.toHaveProperty('timed');
});

it('v3 恢复事件与 Rust 共用同一协议样本', () => {
  const fixture = JSON.parse(readFileSync(new URL('../../../tests/fixtures/protocol/valid-source-recovery.json', import.meta.url), 'utf8'));
  const observation: HarnessObservation = {
    type: 'session/recovered', sessionId: 's', sessionCreatedAt: 100, time: 100, seq: 3, turn: 2,
    observation: fixture.observation,
    recoverySnapshot: { snapshotEventCount: 4, running: true, timed: [{ requestId: 'question:q', turnId: '1', birthSequence: 2 }] },
  };
  expect(toPetEvents(observation, harnessIdentity(observation, 'install-recovery'))).toEqual([fixture]);
});

it('只把已证旧批次和首条新写之后的出生归给对应写者', () => {
  const proof = {
    version: 2 as const, streamId: 'session', epochId: 'new', generation: 2,
    recoveredEventCount: 8, firstLiveSequence: 10,
    ancestors: [{ epochId: 'old', generation: 1, firstSequence: 2,
      verifiedBirths: { firstSequence: 2, eventCount: 4 } }],
  };
  expect(timedRequestId(proof, 'call', 1, 10)).toBe('question:call');
  expect(timedRequestId(proof, 'call', 3, 10)).toBe('question:old:3:call');
  expect(timedRequestId(proof, 'call', 4, 10)).toBeUndefined();
  expect(timedRequestId(proof, 'call', 9, 10)).toBeUndefined();
  expect(timedRequestId(proof, 'call', 10, 10)).toBe('question:new:10:call');
});

it('v1 沿用本代实时问题标识，祖先和撕裂尾归属保留未知', () => {
  const proof = {
    version: 1 as const, streamId: 'session', epochId: 'live', generation: 2, recoveredEventCount: 8,
    ancestors: [{ epochId: 'old', generation: 1, firstSequence: 2 }],
  };
  expect(timedRequestId(proof, 'call', 1, 10)).toBe('question:call');
  expect(timedRequestId(proof, 'call', 3, 10)).toBeUndefined();
  expect(timedRequestId(proof, 'call', 9, 10)).toBeUndefined();
  expect(timedRequestId(proof, 'call', 10, 10)).toBe('question:live:10:call');
});
