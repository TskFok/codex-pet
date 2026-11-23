import type { AlphaMask } from '../animation/hit-mask';
import { reasonName, sourceName, historyName } from '../locales/zh-CN';
import { keyId } from './notifications';
import type { FrameRect, NotificationRecord, PetSnapshot, SessionProjection, Source } from '../types';

interface Presentation {
  state: string;
  title?: string;
  detail?: string;
}
export function presentation(snap: PetSnapshot, muted: Source[]): Presentation {
  const ends = new Map<string, NotificationRecord>();
  for (const n of snap.notifications) if (!n.read && n.kind.type === 'turn_ended') ends.set(keyId(n.key), n);
  const priority = (s: SessionProjection) => {
    if (s.status === 'needs_attention') return 5;
    if (s.status === 'failed') return 4;
    if (ends.has(keyId(s.key)) || (s.unread && ['ended', 'unknown', 'interrupted'].includes(s.status)))
      return 3;
    return s.status === 'running' ? 2 : 0;
  };
  let best: SessionProjection | undefined,
    rank = 0;
  for (const s of snap.sessions) {
    if (muted.includes(s.key.source)) continue;
    const value = priority(s);
    if (value && (value > rank || (value === rank && s.lastEventAtMs > best!.lastEventAtMs))) {
      rank = value;
      best = s;
    }
  }
  if (!best) return { state: 'idle' };
  if (rank === 2) return { state: 'running' };
  const end = ends.get(keyId(best.key));
  const title = [
    sourceName[best.key.source],
    best.project?.name || end?.project?.name || best.key.sessionId,
  ].join(' · ');
  if (rank === 5) {
    const only = best.pending.length === 1 ? best.pending[0] : undefined;
    const reason = only ? reasonName[only.reason] : '有待处理请求';
    return {
      state: only?.reason === 'approval' ? 'review' : 'waiting',
      title,
      detail: [reason, only?.summary].filter(Boolean).join(' · '),
    };
  }
  const state = rank === 4 ? 'failed' : 'waving';
  if (rank === 4 && !best.unread && !end) return { state };
  return {
    state,
    title,
    detail: [
      end ? historyName(end.kind) : rank === 4 ? '回合失败' : '收到回合结束提醒',
      end?.summary || best.summary,
    ]
      .filter(Boolean)
      .join(' · '),
  };
}

/** 合并按行有序的图集跨度和气泡矩形，保留二者外部的穿透区域。 */
export function withBubble(mask: AlphaMask, rect: FrameRect): AlphaMask {
  const left = Math.max(0, Math.floor(rect.x)),
    right = Math.min(mask.width, Math.ceil(rect.x + rect.width));
  const top = Math.max(0, Math.floor(rect.y)),
    bottom = Math.min(mask.height, Math.ceil(rect.y + rect.height));
  if (left >= right || top >= bottom) return mask;
  const runs: AlphaMask['runs'] = [];
  let i = 0;
  for (let y = 0; y < mask.height; y++) {
    let l = left,
      r = right,
      pending = y >= top && y < bottom;
    while (i < mask.runs.length && mask.runs[i][0] === y) {
      const span = mask.runs[i++];
      if (pending && r < span[1]) {
        runs.push([y, l, r]);
        pending = false;
      }
      if (pending && span[2] >= l) {
        l = Math.min(l, span[1]);
        r = Math.max(r, span[2]);
      } else runs.push(span);
    }
    if (pending) runs.push([y, l, r]);
  }
  return { ...mask, runs };
}
