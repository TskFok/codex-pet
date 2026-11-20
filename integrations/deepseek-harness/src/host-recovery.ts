import type { Context } from '@deepseek-ai/cordis';
import type { Session } from '@deepseek-ai/dsh-session';
import type {} from '@deepseek-ai/dsh-session-projection';
import type {} from '@deepseek-ai/dsh-user-questions/types';
import { z } from 'zod';
import type { HarnessObservation, ObservationGeneration, RecoveredQuestion } from './events.js';
import type { QuestionBirths } from './host-births.js';

const position = z.number().int().nonnegative();
const proofFields = z.object({
  streamId: z.uuid(),
  epochId: z.uuid(),
  generation: position.positive(),
  recoveredEventCount: position,
});
const ancestor = z.object({ epochId: z.uuid(), generation: position.positive(), firstSequence: position });
const proofSchema = z.discriminatedUnion('version', [
  proofFields.extend({ version: z.literal(1), ancestors: z.array(ancestor).max(128) }),
  proofFields.extend({
    version: z.literal(2), firstLiveSequence: position,
    ancestors: z.array(ancestor.extend({
      verifiedBirths: z.object({ firstSequence: position, eventCount: position }).optional(),
    })).max(128),
  }),
]);
const proofCache = new WeakMap<object, ObservationGeneration>();

/** 读取补丁宿主公开的唯一写所有者证明，旧 SDK 不需要导入新声明。 */
export function observationOf(owner: object): ObservationGeneration | undefined {
  if (!('observationRecovery' in owner) || typeof owner.observationRecovery !== 'object' || owner.observationRecovery === null) {
    return undefined;
  }
  const previous = proofCache.get(owner.observationRecovery);
  if (previous !== undefined) return previous;
  const parsed = proofSchema.safeParse(owner.observationRecovery);
  if (!parsed.success) return undefined;
  proofCache.set(owner.observationRecovery, parsed.data);
  return parsed.data;
}

function legacyBirthCut(proof: ObservationGeneration): number {
  const first = proof.ancestors[0];
  return first === undefined ? proof.recoveredEventCount : first.generation === 1 ? first.firstSequence : 0;
}

function firstLiveSequence(proof: ObservationGeneration, sessionFirstLive: number): number {
  return proof.firstLiveSequence ?? Math.max(sessionFirstLive, proof.recoveredEventCount);
}

/** 定时问题的身份在保留的日志前缀内沿用出生写入代次。 */
export function timedRequestId(proof: ObservationGeneration, callId: string, birthSequence: number, sessionFirstLive: number): string | undefined {
  if (birthSequence >= firstLiveSequence(proof, sessionFirstLive)) return `question:${proof.epochId}:${birthSequence}:${callId}`;
  for (const owner of proof.ancestors) {
    const range = owner.verifiedBirths;
    if (range !== undefined && birthSequence >= range.firstSequence && birthSequence < range.eventCount) {
      return `question:${owner.epochId}:${birthSequence}:${callId}`;
    }
  }
  return birthSequence < legacyBirthCut(proof) ? `question:${callId}` : undefined;
}

/** 同步采集单个会话的快照，缺少定时问题证明时省略整个覆盖域。 */
export function recoverySnapshot(
  ctx: Context, session: Session, proof: ObservationGeneration, births: QuestionBirths,
): Pick<HarnessObservation, 'observation' | 'type' | 'seq' | 'turn' | 'recoverySnapshot'> {
  const snapshotEventCount = session.seq;
  const seq = snapshotEventCount > 0 ? snapshotEventCount - 1 : undefined;
  const turn = births.turn(session);
  const view = ctx.sessionProjections.snapshot(session, ['userQuestions']);
  let timed: RecoveredQuestion[] | undefined;
  // v1 接收端会把祖先区间的缺席当作解除。有未知区间时只声明活动回合。
  const completeScope = proof.version !== 1 || legacyBirthCut(proof) >= firstLiveSequence(proof, session.firstLiveSeq);
  if (completeScope && view.asOfSeq === (seq ?? -1)
    && view.values.userQuestions !== undefined && !births.hasUnsettledPtcDispatch(session)) {
    timed = [];
    for (const active of view.values.userQuestions.active) {
      const birth = births.birth(session, active.callId);
      const requestId = birth === undefined ? undefined : timedRequestId(proof, active.callId, birth.seq, session.firstLiveSeq);
      if (birth?.timed !== true || requestId === undefined || birth.seq >= snapshotEventCount) {
        timed = undefined;
        break;
      }
      timed.push({ requestId, birthSequence: birth.seq, ...(birth.turn === null ? {} : { turnId: String(birth.turn) }) });
    }
  }
  return {
    observation: proof,
    type: 'session/recovered',
    seq,
    turn,
    recoverySnapshot: { snapshotEventCount, running: turn !== undefined, ...(timed === undefined ? {} : { timed }) },
  };
}
