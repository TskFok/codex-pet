import type { AlphaMask } from '../animation/hit-mask';
import { reasonName, sourceName, historyName } from '../locales/zh-CN';
import { keyId } from './notifications';
import type { FrameRect, NotificationRecord, PetSnapshot, SessionKey, SessionProjection, Source } from '../types';

interface Presentation {
  state: string;
  fallbackState: string;
  animationKey?: string;
  title?: string;
  detail?: string;
  key?: SessionKey;
  reminderId?: string;
}
export function presentation(snap: PetSnapshot, muted: Source[] | undefined): Presentation {
  const ends = new Map<string, NotificationRecord>();
  for (const n of snap.notifications) if (!n.read && n.kind.type === 'turn_ended') ends.set(keyId(n.key), n);
  const importedEnds = new Set(snap.importedUnreadEnds?.map(keyId));
  const priority = (s: SessionProjection) => {
    if (s.status === 'needs_attention') return 5;
    if (s.status === 'failed') return 4;
    if (
      importedEnds.has(keyId(s.key)) ||
      ends.has(keyId(s.key)) ||
      (s.unread && ['ended', 'unknown', 'interrupted'].includes(s.status))
    )
      return 3;
    return s.status === 'running' ? 2 : 0;
  };
  let animation: SessionProjection | undefined, best: SessionProjection | undefined;
  let animationRank = 0,
    rank = 0;
  let hasRunning = false;
  for (const s of snap.sessions) {
    if (s.status === 'running') hasRunning = true;
    const value = priority(s);
    if (!value) continue;
    if (value > animationRank || (value === animationRank && s.lastEventAtMs > animation!.lastEventAtMs)) {
      animation = s;
      animationRank = value;
    }
    if (!muted || muted.includes(s.key.source)) continue;
    if (value > rank || (value === rank && s.lastEventAtMs > best!.lastEventAtMs)) {
      best = s;
      rank = value;
    }
  }
  const defaultStates = ['idle', 'idle', 'running', 'waving', 'failed', 'waiting'];
  const singleApproval = animation?.pending.length === 1 && animation.pending[0].reason === 'approval';
  const state = animationRank === 5 && singleApproval ? 'review' : defaultStates[animationRank];
  const fallbackState = animationRank === 5 ? state : hasRunning ? 'running' : 'idle';
  let animationKey: string | undefined;
  if (animation && animationRank === 4) {
    animationKey = JSON.stringify(['failed', keyId(animation.key), animation.turnId ?? animation.lastEventAtMs]);
  } else if (animation && animationRank === 3) {
    const animId = keyId(animation.key);
    const animEnd = ends.get(animId);
    animationKey = animEnd
      ? JSON.stringify(['notification', animId, animEnd.id])
      : importedEnds.has(animId)
        ? JSON.stringify(['imported', animId])
        : JSON.stringify(['session', animId, animation.status, animation.turnId ?? animation.lastEventAtMs]);
  }
  if (!best || rank === 2) return { state, fallbackState, animationKey };
  const bestId = keyId(best.key);
  const end = ends.get(bestId);
  const title = [
    sourceName[best.key.source],
    best.project?.name || end?.project?.name || best.key.sessionId,
  ].join(' · ');
  if (rank === 5) {
    const only = best.pending.length === 1 ? best.pending[0] : undefined;
    const reason = only ? reasonName[only.reason] : '有待处理请求';
    return {
      state,
      fallbackState,
      animationKey,
      key: best.key,
      reminderId: JSON.stringify(['attention', bestId, best.pending.map((p) => p.id)]),
      title,
      detail: [reason, only?.summary].filter(Boolean).join(' · '),
    };
  }
  if (rank === 4 && !best.unread && !end) return { state, fallbackState, animationKey };
  const reminderId = end
    ? JSON.stringify(['notification', bestId, end.id])
    : importedEnds.has(bestId)
      ? JSON.stringify(['imported', bestId])
      : JSON.stringify(['session', bestId, best.status, best.turnId ?? best.lastEventAtMs]);
  return {
    state,
    fallbackState,
    animationKey,
    key: best.key,
    reminderId,
    title,
    detail: [
      end ? historyName(end.kind) : rank === 4 ? '回合失败' : '收到回合结束提醒',
      end?.summary || (best.status === 'running' ? undefined : best.summary),
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
