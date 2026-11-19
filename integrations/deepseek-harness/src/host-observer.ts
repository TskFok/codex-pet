import { randomUUID } from 'node:crypto';
import type { Context } from '@deepseek-ai/cordis';
import type { Session } from '@deepseek-ai/dsh-session';
import type {} from '@deepseek-ai/dsh-user-approval/types';
import type {} from '@deepseek-ai/dsh-user-questions/types';
import type { QuestionBirths } from './host-births.js';
import type { HarnessObservation } from './events.js';
import { hasDirectQuestions, observeDirectQuestions } from './host-direct.js';
import { observationOf, recoverySnapshot, timedRequestId } from './host-recovery.js';

/** 接收挑选后的字段；绝不接收提示词、问题内容、工具参数或答案。 */
export type ObservationSink = (observation: HarnessObservation) => void | Promise<void>;

/** 透明观察正式Host事件；只使用日志真实seq，不为waterfall编造序号。 */
export function observeHost(ctx: Context, sink: ObservationSink, readTurn?: (session: Session) => number|undefined, births?: QuestionBirths) {
  const turns = new WeakMap<Session, number>();
  const publish = (observation: HarnessObservation): void => {
    try { void Promise.resolve(sink(observation)).catch((_error: unknown) => { ctx.logger.warn('桌宠转发失败，请检查bridge路径；宿主处理继续'); }); }
    catch (_error) { ctx.logger.warn('桌宠转发失败，请检查bridge路径；宿主处理继续'); }
  };
  const fields = (session: Session, turn: number|undefined) => ({ sessionId: session.id, sessionCreatedAt: session.header.createdAt, observation: observationOf(session), ...(turn===undefined?{}:{turn}),
    ...(session.header.origin === 'subagent' && session.header.parentSession !== undefined ? { parentSessionId: session.header.parentSession } : {}) });
  const turnOf = (session: Session): number|undefined => readTurn === undefined ? turns.get(session) : readTurn(session);
  ctx.on('session/event', (session, event) => {
    if (event.seq < session.inheritedEventCount) return;
    if (event.type === 'turn/start') turns.set(session, event.data.turn);
    if (event.type === 'turn/end' && turns.get(session)===event.data.turn) turns.delete(session);
    const base = { ...fields(session, turnOf(session)), seq: event.seq, time: event.time };
    const closed = births?.closed(session,event.seq);
    if (closed !== undefined) {
      const timed = base.observation !== undefined && closed.timed;
      const requestId = timed ? timedRequestId(base.observation!, closed.callId, closed.seq) : closed.requestId;
      if (requestId !== undefined) publish({
        ...base, turn: closed.turn ?? undefined, type: 'question/resolved', requestId,
        ...(timed ? { questionBirth: closed.seq } : {}),
      });
    }
    switch (event.type) {
      case 'turn/start': publish({ ...base, type: 'turn/start' }); break;
      case 'turn/end': {
        const reason = event.data.reason.kind;
        publish({ ...base, type: 'turn/end', turn: event.data.turn, outcome: reason === 'completed' ? 'completed' : reason === 'error' ? 'failed' : reason === 'aborted' || reason === 'interrupted' || reason === 'forked' ? 'interrupted' : 'unknown' }); break;
      }
      case 'approval/asked': publish({ ...base, type: 'approval/asked', requestId: event.data.id }); break;
      case 'approval/decided': publish({ ...base, type: 'approval/decided', requestId: event.data.id }); break;
      default: break; // SessionEventMap允许其他插件扩展；不转发原始数据。
    }
  }, { global: true });
  const publishRecovery = (session: Session): void => {
    try {
      const proof = observationOf(session);
      if (proof === undefined || births === undefined) return;
      publish({ ...fields(session, undefined), ...recoverySnapshot(ctx, session, proof, births), time: session.header.createdAt });
    } catch (_error) {
      ctx.logger.warn('桌宠无法读取会话恢复范围，原会话继续');
    }
  };
  ctx.on('session/created', publishRecovery, { global: true });
  for (const session of ctx.sessions.list()) publishRecovery(session);
  ctx.on('approval/request', (_request, next) => next(), { prepend: true, global: true });
  ctx.on('user-questions/request', async (request, next) => {
    if (request.wait === undefined && hasDirectQuestions(ctx.get('userQuestions'))) return next();
    const session = request.agent === undefined ? undefined : ctx.sessions.get(request.agent.id);
    if (session === undefined) return next();
    const callId = request.wait?.callId;
    const birth = callId === undefined ? undefined : births?.birth(session, callId);
    const proof = observationOf(session);
    const timed = proof !== undefined && birth?.timed === true && callId !== undefined;
    const requestId = timed ? timedRequestId(proof, callId, birth.seq) : `question:${callId ?? randomUUID()}`;
    if (requestId === undefined) return next();
    const observationId = timed ? `timed:${birth.seq}` : randomUUID();
    const base = { ...fields(session, birth === undefined ? turnOf(session) : birth.turn ?? undefined), requestId, observationId,
      ...(timed ? { questionBirth: birth.seq } : {}),
      lifetime:'until_resolved' as const,
      reason: request.questions.some(question=>question.intent?.kind==='approval')?'approval' as const:'answer' as const };
    let resolved = false;
    const resolve = (): void => {
      if (resolved) return;
      resolved = true;
      publish({ ...base, observationId: `${observationId}:resolved`, type: 'question/resolved', time: Date.now() });
    };
    const abort = (): void => {
      // timed超时仍可回答，不能当作用户取消。
      const reason: unknown = request.signal?.reason;
      if (typeof reason === 'object' && reason !== null && 'code' in reason && reason.code === 'ASK_TIMED_OUT') return;
      resolve();
    };
    publish({ ...base, type: 'question/requested', time: Date.now() });
    request.signal?.addEventListener('abort', abort, { once: true });
    if (request.signal?.aborted) abort();
    try { const answer = await next(); resolve(); return answer; }
    catch (error) {
      const timed = request.wait?.timed === true && typeof error === 'object' && error !== null && 'code' in error && (error.code === 'ASK_TIMED_OUT' || error.code === 'NO_PROVIDER');
      if (!timed) resolve();
      throw error;
    } finally { request.signal?.removeEventListener('abort', abort); }
  }, { prepend: true, global: true });
  return observeDirectQuestions(ctx, sink);
}
