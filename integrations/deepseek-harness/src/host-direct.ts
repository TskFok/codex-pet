import { setTimeout as delay } from 'node:timers/promises';
import type { Context } from '@deepseek-ai/cordis';
import type { ObservationSink } from './host-observer.js';
import type { HarnessObservation } from './events.js';

interface DirectQuestion {
  readonly id: string;
  readonly sessionId: string;
  readonly sessionCreatedAt: number;
  readonly birthTurn: number | null;
  readonly reason: 'answer' | 'approval';
  readonly startedAt: number;
}
type DirectChange =
  | { readonly kind: 'started'; readonly cursor: number; readonly request: DirectQuestion }
  | { readonly kind: 'settled'; readonly cursor: number; readonly request: DirectQuestion; readonly settledAt: number };
type Settled = Extract<DirectChange, { kind: 'settled' }>;
interface DirectQuestionServiceV1 {
  readonly directQuestionObservationVersion: 1;
  observeDirectQuestions(listener: (change: DirectChange) => void): {
    readonly snapshot: {
      readonly version: 1;
      readonly cursor: number;
      readonly settledAfter: number;
      readonly active: readonly DirectQuestion[];
      readonly settled: readonly Settled[];
    };
    dispose(): void;
  };
}

/** 用宿主公开能力版本协商，兼容没有恢复接口的正式 SDK。 */
export function hasDirectQuestions(service: unknown): service is DirectQuestionServiceV1 {
  return typeof service === 'object' && service !== null
    && 'directQuestionObservationVersion' in service && service.directQuestionObservationVersion === 1
    && 'observeDirectQuestions' in service && typeof service.observeDirectQuestions === 'function';
}

const LIVE_LIMIT = 2_000;
const RETRY_DELAY_MS = 500;

type Snapshot = ReturnType<DirectQuestionServiceV1['observeDirectQuestions']>['snapshot'];

function* recover(snapshot: Snapshot, deliveredThrough?: number): Iterable<HarnessObservation> {
  for (const change of snapshot.settled) {
    if (deliveredThrough === undefined || change.cursor > deliveredThrough) {
      yield observation(change.request, change.settledAt);
    }
  }
  for (const request of snapshot.active) yield observation(request);
}

function observation(request: DirectQuestion, settledAt?: number): HarnessObservation {
  return {
    sessionId: request.sessionId,
    sessionCreatedAt: request.sessionCreatedAt,
    ...(request.birthTurn === null ? {} : { turn: request.birthTurn }),
    type: settledAt === undefined ? 'question/requested' : 'question/resolved',
    requestId: `question:${request.id}`,
    observationId: settledAt === undefined ? request.id : `${request.id}:resolved`,
    reason: request.reason,
    lifetime: 'until_resolved',
    time: settledAt ?? request.startedAt,
  };
}

function pumpDirectQuestions(service: DirectQuestionServiceV1, send: ObservationSink, warn: (message: string) => void) {
  const pending: DirectChange[] = [];
  const stop = new AbortController();
  let task: Promise<void> | undefined;
  let unsubscribe: (() => void) | undefined;
  let baseline: Iterator<HarnessObservation> | undefined;
  let deliveredThrough = 0;
  let baselineCut = 0;
  let overflow = false;

  function capture(after?: number): void {
    unsubscribe?.();
    const { snapshot, dispose } = service.observeDirectQuestions(change => {
      if (overflow || stop.signal.aborted) return;
      if (pending.length === LIVE_LIMIT) {
        overflow = true;
        warn('桌宠直接提问增量积压，将重新读取宿主恢复快照');
      } else {
        pending.push(change);
      }
      schedule();
    });
    unsubscribe = dispose;
    if (snapshot.version !== 1) {
      dispose();
      return;
    }
    if (after !== undefined && snapshot.settledAfter > after) {
      warn('桌宠直接提问的结束审计已超出宿主覆盖范围，部分请求仍待核实');
    }
    baseline = recover(snapshot, after)[Symbol.iterator]();
    baselineCut = snapshot.cursor;
  }

  async function deliver(event: HarnessObservation): Promise<boolean> {
    let warned = false;
    for (;;) {
      try {
        await send(event);
        return true;
      } catch (_error) {
        if (!warned) {
          warn('桌宠直接提问尚未接纳，将重试同一事件，请检查桥接程序和待发送区');
          warned = true;
        }
      }
      if (stop.signal.aborted) return false;
      await delay(RETRY_DELAY_MS, undefined, { signal: stop.signal }).catch((error: unknown) => {
        if (!stop.signal.aborted) throw error;
      });
      if (stop.signal.aborted) return false;
    }
  }

  async function drain(): Promise<void> {
    while (!stop.signal.aborted) {
      if (baseline !== undefined) {
        const next = baseline.next();
        if (!next.done) {
          if (!await deliver(next.value)) return;
          continue;
        }
        baseline = undefined;
        deliveredThrough = baselineCut;
      }
      const change = pending.shift();
      if (change !== undefined) {
        if (!await deliver(observation(change.request, change.kind === 'settled' ? change.settledAt : undefined))) return;
        deliveredThrough = change.cursor;
        continue;
      }
      if (!overflow) return;
      overflow = false;
      capture(deliveredThrough);
    }
  }

  function schedule(): void {
    if (task !== undefined) return;
    task = Promise.resolve().then(drain).catch((_error: unknown) => {
      warn('桌宠直接提问观察中断，请重载观察器恢复');
    }).finally(() => {
      task = undefined;
      if (!stop.signal.aborted && pending.length > 0) schedule();
    });
  }

  capture();
  schedule();
  return async (): Promise<void> => {
    stop.abort();
    unsubscribe?.();
    await task;
  };
}

/** 有回压地恢复宿主事实，原提问流程只投递观察信号。 */
export function observeDirectQuestions(ctx: Context, send: ObservationSink) {
  let close: (() => Promise<void>) | undefined;
  const observer = ctx.inject(['userQuestions'], child => {
    const service = child.get('userQuestions');
    if (!hasDirectQuestions(service)) return;
    child.effect(() => {
      close = pumpDirectQuestions(service, send, message => child.logger.warn(message));
      return close;
    }, 'desktop-pet:direct-questions');
  });
  return {
    ready: ctx.get('userQuestions') === undefined ? undefined : Promise.resolve(observer).then(() => undefined),
    async dispose(): Promise<void> {
      // Cordis 会并行启动同级 effect 清理。等待恢复本身结束后，外层才能关闭转发器。
      await close?.();
      await observer.dispose();
    },
  };
}
