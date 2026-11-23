import type { NotificationKind, SessionProjection } from '../types';
export const sourceName = { codex: 'Codex', deepseek: 'DeepSeek Harness' };
export const reasonName = { answer: '待回答', approval: '待审批' };
export const levelName = { verified: '已验证', unsupported: '不支持', unverified: '未验证' };
export function statusName(s: SessionProjection): string {
  if (s.status === 'running' && s.key.source === 'codex') return '最近观测到回合开始';
  return {
    running: '工作中',
    needs_attention: '待处理',
    ended: '回合已结束',
    failed: '回合失败',
    interrupted: '回合中断',
    unknown: '当前状态待核实',
  }[s.status];
}
export function historyName(k: NotificationKind): string {
  if (k.type === 'attention')
    return k.resolved
      ? k.reason === 'approval'
        ? '审批提醒已解除'
        : '提问提醒已解除'
      : k.reason === 'approval'
        ? '曾请求审批'
        : '曾请求回答';
  return {
    unknown: '回合已结束，结果未知',
    completed: '回合已结束',
    failed: '回合失败',
    interrupted: '回合中断',
  }[k.outcome];
}
