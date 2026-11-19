export const SCHEMA_VERSION = 2 as const;
export const RECOVERY_SCHEMA_VERSION = 3 as const;
export type RequestLifetime = 'turn' | 'until_resolved';

export interface SourceIdentity { source: 'deepseek'; installationId: string; instanceId: string }
export interface ObservationGeneration {
  version: 1;
  streamId: string;
  epochId: string;
  generation: number;
  recoveredEventCount: number;
  ancestors: readonly { epochId: string; generation: number; firstSequence: number }[];
}
export interface RecoveredQuestion { requestId: string; turnId?: string; birthSequence: number }
interface RecoverySnapshot { snapshotEventCount: number; running: boolean; timed?: readonly RecoveredQuestion[] }
export interface HarnessObservation {
  sessionId: string;
  sessionCreatedAt: number;
  parentSessionId?: string;
  turn?: number;
  seq?: number;
  observationId?: string;
  observation?: ObservationGeneration;
  questionBirth?: number;
  recoverySnapshot?: RecoverySnapshot;
  time: number;
  type: 'turn/start' | 'turn/end' | 'approval/asked' | 'approval/decided' | 'question/requested' | 'question/resolved' | 'session/recovered';
  requestId?: string;
  lifetime?: RequestLifetime;
  reason?: 'answer' | 'approval';
  outcome?: 'completed' | 'interrupted' | 'failed' | 'unknown';
}
export interface EventEnvelope {
  schemaVersion: 2 | 3;
  eventId: string;
  identity: SourceIdentity;
  sessionId: string;
  parentSessionId?: string;
  turnId?: string;
  upstreamEventId?: string;
  sequence?: number;
  observation?: ObservationGeneration;
  questionBirth?: number;
  occurredAtMs: number;
  summary?: string;
  kind:
    | { type: 'turn_started' }
    | { type: 'turn_ended'; outcome: 'completed' | 'interrupted' | 'failed' | 'unknown' }
    | { type: 'attention_requested'; requestId: string; reason: 'answer' | 'approval'; lifetime?: RequestLifetime }
    | { type: 'attention_resolved'; requestId: string }
    | ({ type: 'source_recovery' } & RecoverySnapshot);
}

/** 只转发宿主已确认的日志位置、活动范围及脱敏请求标识。 */
export function toPetEvents(observation: HarnessObservation, identity: SourceIdentity): EventEnvelope[] {
  const { type, requestId } = observation;
  let kind: EventEnvelope['kind'];
  switch (type) {
    case 'turn/start': kind = { type: 'turn_started' }; break;
    case 'turn/end': kind = { type: 'turn_ended', outcome: observation.outcome ?? 'unknown' }; break;
    case 'approval/asked':
    case 'question/requested':
      if (!requestId) throw new Error('待处理观察缺少请求标识');
      kind = { type: 'attention_requested', requestId, reason: observation.reason ?? (type === 'approval/asked' ? 'approval' : 'answer'), ...(observation.lifetime === 'until_resolved' ? { lifetime: observation.lifetime } : {}) };
      break;
    case 'approval/decided':
    case 'question/resolved':
      if (!requestId) throw new Error('解除观察缺少请求标识');
      kind = { type: 'attention_resolved', requestId };
      break;
    case 'session/recovered':
      if (observation.observation === undefined || observation.recoverySnapshot === undefined) {
        throw new Error('恢复事件缺少来源证明或覆盖范围');
      }
      kind = { type: 'source_recovery', ...observation.recoverySnapshot };
  }
  const eventId = JSON.stringify([
    identity.installationId, identity.instanceId, observation.sessionId,
    observation.seq ?? observation.observationId, type, requestId ?? null,
    ...(kind.type === 'source_recovery' ? [kind.timed !== undefined] : []),
  ]);
  const envelope: EventEnvelope = {
    schemaVersion: observation.observation === undefined ? SCHEMA_VERSION : RECOVERY_SCHEMA_VERSION,
    eventId,
    identity, sessionId: observation.sessionId,
    ...(observation.parentSessionId === undefined ? {} : { parentSessionId: observation.parentSessionId }),
    ...(observation.turn === undefined ? {} : { turnId: String(observation.turn) }),
    ...(observation.seq === undefined ? {} : { upstreamEventId: `${observation.sessionId}:${observation.seq}`, sequence: observation.seq }),
    ...(observation.observation === undefined ? {} : { observation: observation.observation }),
    ...(observation.questionBirth === undefined ? {} : { questionBirth: observation.questionBirth }),
    occurredAtMs: observation.time, kind,
  };
  if (kind.type === 'source_recovery' && kind.timed !== undefined && Buffer.byteLength(JSON.stringify(envelope)) > 65_536) {
    const { timed: _timed, ...partial } = kind;
    return [{
      ...envelope,
      eventId: JSON.stringify([identity.installationId, identity.instanceId, observation.sessionId, observation.seq, type, null, false]),
      kind: partial,
    }];
  }
  return [envelope];
}

/** 有恢复证明时使用宿主写代次，旧宿主继续用之前的日志观察范围。 */
export function harnessIdentity(observation: Pick<HarnessObservation, 'sessionId' | 'sessionCreatedAt' | 'observation'>, installationId: string): SourceIdentity {
  return {
    source: 'deepseek',
    installationId,
    instanceId: observation.observation?.epochId ?? `session-log:${observation.sessionId}:${observation.sessionCreatedAt}`,
  };
}
