import type { Context } from '@deepseek-ai/cordis';
import type { Session } from '@deepseek-ai/dsh-session';
import type {} from '@deepseek-ai/dsh-session-projection';
import type {} from '@deepseek-ai/dsh-user-questions/types';
import { z } from 'zod';
import type { HarnessObservation, ObservationGeneration, RecoveredQuestion } from './events.js';
import type { QuestionBirths } from './host-births.js';

const position = z.number().int().nonnegative();
const proofSchema = z.object({
  version: z.literal(2),
  firstLiveSequence: position,
  streamId: z.uuid(),
  epochId: z.uuid(),
  generation: position.positive(),
  recoveredEventCount: position,
  ancestors: z.array(z.object({
    epochId: z.uuid(), generation: position.positive(), firstSequence: position,
    verifiedBirths: z.object({ firstSequence: position, eventCount: position }).optional(),
  })).max(128),
});
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

/** 定时问题的身份在保留的日志前缀内沿用出生写入代次。 */
export function timedRequestId(proof: ObservationGeneration, callId: string, birthSequence: number): string | undefined {
  if (proof.firstLiveSequence === undefined) return undefined;
  if (birthSequence >= proof.firstLiveSequence) return `question:${proof.epochId}:${birthSequence}:${callId}`;
  for (const owner of proof.ancestors) {
    const range = owner.verifiedBirths;
    if (range !== undefined && birthSequence >= range.firstSequence && birthSequence < range.eventCount) {
      return `question:${owner.epochId}:${birthSequence}:${callId}`;
    }
  }
  const first = proof.ancestors[0];
  const legacyCut = first === undefined ? proof.recoveredEventCount : first.generation === 1 ? first.firstSequence : 0;
  return birthSequence < legacyCut ? `question:${callId}` : undefined;
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
  if (view.asOfSeq === (seq ?? -1) && view.values.userQuestions !== undefined && !births.hasUnsettledPtcDispatch(session)) {
    timed = [];
    for (const active of view.values.userQuestions.active) {
      const birth = births.birth(session, active.callId);
      const requestId = birth === undefined ? undefined : timedRequestId(proof, active.callId, birth.seq);
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
