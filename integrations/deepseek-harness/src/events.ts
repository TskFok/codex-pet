export const SCHEMA_VERSION = 2 as const;
export type RequestLifetime = 'turn' | 'until_resolved';
export interface SourceIdentity { source: 'deepseek'; installationId: string; instanceId: string }
export interface HarnessObservation { sessionId: string; sessionCreatedAt: number; parentSessionId?: string; turn?: number; seq?: number; observationId?: string; time: number; type: 'turn/start' | 'turn/end' | 'approval/asked' | 'approval/decided' | 'question/requested' | 'question/resolved'; requestId?: string; lifetime?: RequestLifetime; reason?: 'answer'|'approval'; outcome?: 'completed' | 'interrupted' | 'failed' | 'unknown' }
export interface EventEnvelope { schemaVersion: 2; eventId: string; identity: SourceIdentity; sessionId: string; parentSessionId?: string; turnId?: string; upstreamEventId?: string; sequence?: number; occurredAtMs: number; summary?: string; kind: { type: 'turn_started' } | { type: 'turn_ended'; outcome: 'completed'|'interrupted'|'failed'|'unknown' } | { type: 'attention_requested'; requestId: string; reason: 'answer'|'approval'; lifetime?: RequestLifetime } | { type: 'attention_resolved'; requestId: string } }
/** 将挑选后的宿主状态转换为桌宠已有JSON协议；seq始终来自会话日志。 */
export function toPetEvents(observation: HarnessObservation, identity: SourceIdentity): EventEnvelope[] {
  const { type, requestId } = observation;
  let kind: EventEnvelope['kind'];
  switch (type) {
    case 'turn/start': kind = { type: 'turn_started' }; break;
    case 'turn/end': kind = { type: 'turn_ended', outcome: observation.outcome ?? 'unknown' }; break;
    case 'approval/asked':
    case 'question/requested':
      if (!requestId) throw new Error('待处理观察缺少请求标识');
      kind = { type: 'attention_requested', requestId, reason: observation.reason ?? (type === 'approval/asked' ? 'approval' : 'answer'), ...(observation.lifetime === 'until_resolved' ? {lifetime: observation.lifetime} : {}) }; break;
    case 'approval/decided':
    case 'question/resolved':
      if (!requestId) throw new Error('解除观察缺少请求标识');
      kind = { type: 'attention_resolved', requestId }; break;
  }
  return [{ schemaVersion: SCHEMA_VERSION,
    eventId: JSON.stringify([identity.installationId, identity.instanceId, observation.sessionId, observation.seq ?? observation.observationId, type, requestId ?? null]),
    identity, sessionId: observation.sessionId,
    ...(observation.parentSessionId === undefined ? {} : { parentSessionId: observation.parentSessionId }),
    ...(observation.turn === undefined ? {} : {turnId: String(observation.turn)}),
    ...(observation.seq === undefined ? {} : { upstreamEventId: `${observation.sessionId}:${observation.seq}`, sequence: observation.seq }),
    occurredAtMs: observation.time, kind }];
}

/** 实例标识表示同一持久逻辑日志，不声明宿主进程存活或崩溃尾部耐久性。 */
export function harnessIdentity(observation: Pick<HarnessObservation,'sessionId'|'sessionCreatedAt'>, installationId:string):SourceIdentity {
  return {source:'deepseek',installationId,instanceId:`session-log:${observation.sessionId}:${observation.sessionCreatedAt}`};
}
