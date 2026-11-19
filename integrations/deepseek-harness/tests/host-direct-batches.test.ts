import { Context } from '@deepseek-ai/cordis';
import { expect, it } from 'vitest';
import { observeDirectQuestions } from '../src/host-direct.js';
import type { HarnessObservation } from '../src/events.js';

function request(id: string) {
  return { id, sessionId: 's', sessionCreatedAt: 100, birthTurn: null, reason: 'answer', startedAt: 101 } as const;
}

type Change = { kind: 'settled'; cursor: number; request: ReturnType<typeof request>; settledAt: number };

it('大于转发容量的恢复批次会完整接纳，切点后的解除按宿主顺序交接', async () => {
  const ctx = new Context();
  const active = Array.from({ length: 1_100 }, (_, index) => request(`active-${index}`));
  const settled: Change[] = Array.from({ length: 1_024 }, (_, index) => ({
    kind: 'settled', cursor: index + 1, request: request(`past-${index}`), settledAt: 102,
  }));
  let listener: (change: Change) => void = () => {};
  await ctx.plugin({ apply(host: Context) {
    return host.provide('userQuestions', {
      directQuestionObservationVersion: 1,
      observeDirectQuestions(next: (change: Change) => void) {
        listener = next;
        return { snapshot: { version: 1, cursor: 2_124, settledAfter: 0, active, settled }, dispose() {} };
      },
    });
  } });
  const first = Promise.withResolvers<void>();
  const finished = Promise.withResolvers<void>();
  const accepted: HarnessObservation[] = [];
  let inFlight = 0;
  let maxInFlight = 0;
  try {
    await observeDirectQuestions(ctx, async event => {
      maxInFlight = Math.max(maxInFlight, ++inFlight);
      if (event.requestId === 'question:past-0') await first.promise;
      accepted.push(event);
      inFlight--;
      if (accepted.length === 2_125) finished.resolve();
    }).ready;
    listener({ kind: 'settled', cursor: 2_125, request: active.at(-1)!, settledAt: 110 });
    expect(maxInFlight).toBe(1);
    expect(accepted).toHaveLength(0);
    first.resolve();
    await finished.promise;
    expect(accepted).toHaveLength(2_125);
    expect(accepted.slice(-2).map(event => [event.type, event.requestId])).toEqual([
      ['question/requested', 'question:active-1099'],
      ['question/resolved', 'question:active-1099'],
    ]);
    expect(new Set(accepted.map(event => event.observationId)).size).toBe(2_125);
  } finally {
    first.resolve();
    await ctx.fiber.dispose();
  }
});

it('转发暂未接纳时重试同一事件，之后继续交付解除', async () => {
  const ctx = new Context();
  const question = request('retry');
  let listener: (change: Change) => void = () => {};
  await ctx.plugin({ apply(host: Context) {
    return host.provide('userQuestions', {
      directQuestionObservationVersion: 1,
      observeDirectQuestions(next: (change: Change) => void) {
        listener = next;
        return { snapshot: { version: 1, cursor: 1, settledAfter: 0, active: [question], settled: [] }, dispose() {} };
      },
    });
  } });
  const rejected = Promise.withResolvers<void>();
  const delivered = Promise.withResolvers<void>();
  const attempts: string[] = [];
  try {
    await observeDirectQuestions(ctx, event => {
      attempts.push(event.observationId!);
      if (attempts.length === 1) {
        rejected.resolve();
        throw new Error('未接纳');
      }
      if (event.type === 'question/resolved') delivered.resolve();
    }).ready;
    await rejected.promise;
    listener({ kind: 'settled', cursor: 2, request: question, settledAt: 110 });
    await delivered.promise;
    expect(attempts).toEqual(['retry', 'retry', 'retry:resolved']);
  } finally {
    await ctx.fiber.dispose();
  }
});
