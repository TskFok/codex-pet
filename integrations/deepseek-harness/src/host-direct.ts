import type { Context } from '@deepseek-ai/cordis';
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
  | { readonly kind: 'started'; readonly request: DirectQuestion }
  | { readonly kind: 'settled'; readonly request: DirectQuestion; readonly settledAt: number };
type Settled = Extract<DirectChange, { kind: 'settled' }>;
interface DirectQuestionServiceV1 {
  readonly directQuestionObservationVersion: 1;
  observeDirectQuestions(listener: (change: DirectChange) => void): {
    readonly snapshot: {
      readonly version: 1;
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

/** 在宿主服务生命周期内接收精确终态并复用原请求标识。 */
export function observeDirectQuestions(ctx: Context, publish: (observation: HarnessObservation) => void): void | Promise<void> {
  const observer = ctx.inject(['userQuestions'], child => {
    const service = child.get('userQuestions');
    if (!hasDirectQuestions(service)) return;
    child.effect(() => {
      const emit = (change: DirectChange): void => {
        const { request } = change;
        const settled = change.kind === 'settled';
        publish({
          sessionId: request.sessionId,
          sessionCreatedAt: request.sessionCreatedAt,
          ...(request.birthTurn === null ? {} : { turn: request.birthTurn }),
          type: settled ? 'question/resolved' : 'question/requested',
          requestId: `question:${request.id}`,
          observationId: settled ? `${request.id}:resolved` : request.id,
          reason: request.reason,
          lifetime: 'until_resolved',
          time: settled ? change.settledAt : request.startedAt,
        });
      };
      const { snapshot, dispose } = service.observeDirectQuestions(emit);
      if (snapshot.version !== 1) {
        dispose();
        return () => {};
      }
      for (const change of snapshot.settled) emit(change);
      for (const request of snapshot.active) emit({ kind: 'started', request });
      return dispose;
    }, 'desktop-pet:direct-questions');
  });
  if (ctx.get('userQuestions') !== undefined) return Promise.resolve(observer).then(() => undefined);
}
