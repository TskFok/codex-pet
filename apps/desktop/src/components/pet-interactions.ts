import type { InteractionEffect } from '../animation/effects';
import type { PetSnapshot, PetSoundCue } from '../types';
import { PET_STAGE_HEIGHT, PET_STAGE_WIDTH, PET_WINDOW_HEIGHT, PET_WINDOW_WIDTH } from './pet';
import { isSubagentSession } from './reminder';

export interface PetClickEvent {
  x: number;
  y: number;
  clickCount?: number;
  altKey?: boolean;
  shiftKey?: boolean;
}

export type PetHitZone = 'head' | 'body-left' | 'body-center' | 'body-right' | 'feet';

export type InteractionBubbleAction = 'pet_feed' | 'pet_play' | 'pet_notifications';

export interface InteractionStep {
  state: string;
  durationMs?: number;
}

export interface InteractionBubble {
  id: string;
  title: string;
  detail: string;
  actions: InteractionBubbleAction[];
  expiresAtMs: number;
}

export interface InteractionOutcome {
  steps?: InteractionStep[];
  effect?: InteractionEffect;
  sound?: PetSoundCue;
  bubble?: InteractionBubble;
  openNotifications?: boolean;
  closeBubble?: boolean;
}

export interface PetInteractionStats {
  affinity: number;
  interactions: number;
  feeds: number;
}

export interface InteractionContext {
  petName: string;
  snapshot?: PetSnapshot;
  notifySubagents?: boolean;
}

export const INTERACTION_ACTION_LABELS: Record<InteractionBubbleAction, string> = {
  pet_feed: '投喂',
  pet_play: '玩耍',
  pet_notifications: '通知',
};

const STORAGE_KEY = 'codex-pet:interaction-stats:v1';
const MULTI_CLICK_MS = 350;
const MULTI_CLICK_DIST = 0.1;
const POKE_WINDOW_MS = 2800;
const DIZZY_THRESHOLD = 6;
const DIZZY_COOLDOWN_MS = 2600;
const DRAG_REACT_COOLDOWN_MS = 8000;
const QUIP_DURATION_MS = 2800;
const PANEL_DURATION_MS = 8500;

const STAGE_LEFT = (PET_WINDOW_WIDTH - PET_STAGE_WIDTH) / 2 / PET_WINDOW_WIDTH;
const STAGE_WIDTH = PET_STAGE_WIDTH / PET_WINDOW_WIDTH;
const STAGE_TOP = (PET_WINDOW_HEIGHT - PET_STAGE_HEIGHT) / PET_WINDOW_HEIGHT;
const STAGE_HEIGHT = PET_STAGE_HEIGHT / PET_WINDOW_HEIGHT;

const HEAD_LINES = [
  '被摸头了，好开心~',
  '呼噜呼噜，充满干劲！',
  '再多摸一会儿嘛 ✨',
  '收到摸头鼓励，继续加油！',
];

const BODY_CENTER_LINES = [
  '在呢在呢，随时待命！',
  '双击可以打开互动面板哦~',
  '我正帮你盯着工作进度呢！',
  '轻轻戳一下就好啦~',
];

const BODY_LEFT_LINES = [
  '诶嘿，向右闪避 →',
  '别戳左边啦，痒痒的~',
  '反应够快吧，抓不到我！',
];

const BODY_RIGHT_LINES = [
  '← 向左闪避成功！',
  '这边也会怕痒的哦~',
  '灵活走位，躲开啦！',
];

const FEET_LINES = [
  '好痒！差点原地起飞啦',
  '别挠脚底板呀~',
  '哇！跳起来躲一躲！',
];

const FEED_LINES = [
  '啊呜~ 点心真好吃！亲密度 +2',
  '能量补满，继续陪你写代码 ✨',
  '谢谢投喂小零食，元气满满！',
];

const PLAY_LINES = [
  '来啦来啦，转圈圈起跳！',
  '热身完毕，活力百分百 ♪',
  '嘿咻！和你一起玩最开心啦~',
];

const DIZZY_LINES = [
  '别再猛戳啦，头要转成小风车了…',
  '好晕好晕，让我缓两秒钟~',
];

const DRAG_LINES = [
  '换个舒服的新位置待着~',
  '新视野不错，这里很稳！',
];

const SHOWCASE_SEQUENCE: Array<{ state: string; label: string; durationMs?: number }> = [
  { state: 'jumping', label: '开心跳跃' },
  { state: 'waving', label: '招手打招呼' },
  { state: 'running-right', label: '向右冲刺', durationMs: 1400 },
  { state: 'running-left', label: '向左冲刺', durationMs: 1400 },
  { state: 'review', label: '认真审阅', durationMs: 1600 },
  { state: 'waiting', label: '乖巧等待', durationMs: 1600 },
];

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

export function classifyHitZone(x: number, y: number): PetHitZone {
  const localX = clamp((x - STAGE_LEFT) / STAGE_WIDTH, 0, 1);
  const localY = clamp((y - STAGE_TOP) / STAGE_HEIGHT, 0, 1);
  if (localY < 0.38) return 'head';
  if (localY >= 0.74) return 'feet';
  if (localX < 0.42) return 'body-left';
  if (localX > 0.58) return 'body-right';
  return 'body-center';
}

export function affinityTier(affinity: number): string {
  if (affinity >= 70) return '形影不离';
  if (affinity >= 30) return '默契搭档';
  if (affinity >= 10) return '渐渐熟络';
  return '初来乍到';
}

function summarizeStatus(
  snapshot: PetSnapshot | undefined,
  stats: PetInteractionStats,
  notifySubagents = true,
): string {
  if (snapshot) {
    let pendingCount = 0;
    let runningCount = 0;
    for (const session of snapshot.sessions) {
      if (!notifySubagents && isSubagentSession(session)) continue;
      if (session.status === 'needs_attention') pendingCount += Math.max(1, session.pending.length);
      else if (session.status === 'running') runningCount += 1;
    }
    if (pendingCount > 0) return `有 ${pendingCount} 项待处理请求 · 已互动 ${stats.interactions} 次`;
    if (runningCount > 0) return `${runningCount} 个会话运行中 · 正在认真值守`;
  }
  return `当前空闲中 · 已互动 ${stats.interactions} 次，投喂 ${stats.feeds} 次`;
}

function loadStats(): PetInteractionStats {
  try {
    if (typeof localStorage === 'undefined') return { affinity: 0, interactions: 0, feeds: 0 };
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { affinity: 0, interactions: 0, feeds: 0 };
    const parsed = JSON.parse(raw) as Partial<PetInteractionStats>;
    return {
      affinity: typeof parsed.affinity === 'number' && Number.isFinite(parsed.affinity) ? Math.max(0, Math.floor(parsed.affinity)) : 0,
      interactions: typeof parsed.interactions === 'number' && Number.isFinite(parsed.interactions) ? Math.max(0, Math.floor(parsed.interactions)) : 0,
      feeds: typeof parsed.feeds === 'number' && Number.isFinite(parsed.feeds) ? Math.max(0, Math.floor(parsed.feeds)) : 0,
    };
  } catch {
    return { affinity: 0, interactions: 0, feeds: 0 };
  }
}

function saveStats(stats: PetInteractionStats): void {
  try {
    if (typeof localStorage === 'undefined') return;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(stats));
  } catch {
    // 忽略受限环境中的本地存储错误
  }
}

export class PetInteractionController {
  private stats: PetInteractionStats;
  private recentClicks: number[] = [];
  private lastClick?: { atMs: number; x: number; y: number; streak: number };
  private dizzyUntilMs = -Infinity;
  private lastDragReactAtMs = -Infinity;
  private bubbleSeq = 0;
  private lineCounters = new Map<string, number>();
  private showcaseIndex = 0;
  private headAlternator = 0;

  constructor(initialStats?: PetInteractionStats) {
    this.stats = initialStats ? { ...initialStats } : loadStats();
  }

  getStats(): PetInteractionStats {
    return { ...this.stats };
  }

  handleClick(event: PetClickEvent, nowMs: number, context: InteractionContext): InteractionOutcome {
    const x = clamp(Number.isFinite(event.x) ? event.x : 0.5, 0, 1);
    const y = clamp(Number.isFinite(event.y) ? event.y : 0.72, 0, 1);

    // 1. Alt/Option + 单击：快捷投喂
    if (event.altKey) {
      this.recordInteraction(2, 1);
      return {
        steps: [{ state: 'jumping' }],
        effect: { x, y, kind: 'snack', bounce: 1.15 },
        sound: 'feed',
        bubble: this.makeBubble(
          `${context.petName} · 收到投喂`,
          this.pickLine('feed', FEED_LINES),
          [],
          nowMs + 3200,
        ),
      };
    }

    // 2. Shift + 单击：动作轮播展示
    if (event.shiftKey) {
      this.recordInteraction(1, 0);
      const item = SHOWCASE_SEQUENCE[this.showcaseIndex % SHOWCASE_SEQUENCE.length];
      this.showcaseIndex += 1;
      return {
        steps: [{ state: item.state, durationMs: item.durationMs }],
        effect: { x, y, kind: 'star', bounce: 0.9 },
        sound: 'showcase',
        bubble: this.makeBubble(
          `${context.petName} · 动作演示`,
          `正在表演：${item.label}`,
          [],
          nowMs + 2600,
        ),
      };
    }

    // 3. 眩晕恢复冷却期内轻柔反馈
    if (nowMs < this.dizzyUntilMs) {
      return {
        effect: { x, y, kind: 'dizzy', bounce: 0.5, count: 2 },
        sound: 'dizzy_recover',
        bubble: this.makeBubble(
          `${context.petName} · 缓一缓`,
          '还在转圈圈呢，稍等我缓一下~',
          [],
          nowMs + 2000,
        ),
      };
    }

    // 4. 统计短时间连戳频率，达到阈值触发眩晕彩蛋
    this.recentClicks = this.recentClicks.filter((ts) => nowMs - ts <= POKE_WINDOW_MS);
    this.recentClicks.push(nowMs);
    if (this.recentClicks.length >= DIZZY_THRESHOLD) {
      this.recentClicks = [];
      this.lastClick = undefined;
      this.dizzyUntilMs = nowMs + DIZZY_COOLDOWN_MS;
      this.recordInteraction(0, 0);
      return {
        steps: [{ state: 'failed' }],
        effect: { x, y, kind: 'dizzy', bounce: 1.3 },
        sound: 'dizzy',
        bubble: this.makeBubble(
          `${context.petName} · 转晕啦`,
          this.pickLine('dizzy', DIZZY_LINES),
          [],
          nowMs + 3400,
        ),
      };
    }

    // 5. 计算连击次数（兼容原生 clickCount 与时间窗口判定）
    const isMultiClick =
      this.lastClick !== undefined &&
      nowMs - this.lastClick.atMs <= MULTI_CLICK_MS &&
      Math.hypot(x - this.lastClick.x, y - this.lastClick.y) <= MULTI_CLICK_DIST;
    const trackedStreak = isMultiClick ? this.lastClick!.streak + 1 : 1;
    const effectiveClickCount = Math.max(event.clickCount ?? 1, trackedStreak);
    this.lastClick = { atMs: nowMs, x, y, streak: effectiveClickCount };

    // 6. 三连击：活力应援连招
    if (effectiveClickCount >= 3) {
      this.recordInteraction(2, 0);
      return {
        steps: [{ state: 'running-right', durationMs: 650 }, { state: 'jumping' }],
        effect: { x, y, kind: 'star', bounce: 1.2, count: 6 },
        sound: 'triple_combo',
        bubble: this.makeBubble(
          `${context.petName} · 三连击！`,
          '收到超强应援！活力瞬间拉满 ✨',
          [],
          nowMs + 3200,
        ),
      };
    }

    // 7. 双击：打开快捷互动面板
    if (effectiveClickCount === 2) {
      this.recordInteraction(1, 0);
      return {
        steps: [{ state: 'jumping' }],
        effect: { x, y, kind: 'sparkle', bounce: 1.05, count: 6 },
        sound: 'double_click',
        bubble: this.makePanelBubble(
          context.petName,
          summarizeStatus(context.snapshot, this.stats, context.notifySubagents ?? true),
          nowMs,
        ),
      };
    }

    // 8. 单击：按命中部位给出不同反应
    const zone = classifyHitZone(x, y);
    if (zone === 'head') {
      this.recordInteraction(1, 0);
      const state = this.headAlternator++ % 2 === 0 ? 'waving' : 'jumping';
      return {
        steps: [{ state }],
        effect: { x, y, kind: 'heart', bounce: 0.85 },
        sound: 'pet_head',
        bubble: this.makeBubble(
          `${context.petName} · 摸摸头`,
          this.pickLine('head', HEAD_LINES),
          [],
          nowMs + QUIP_DURATION_MS,
        ),
      };
    }
    if (zone === 'body-left') {
      this.recordInteraction(1, 0);
      return {
        steps: [{ state: 'running-right', durationMs: 950 }],
        effect: { x, y, kind: 'note', bounce: 0.95 },
        sound: 'dodge',
        bubble: this.makeBubble(
          `${context.petName} · 躲闪`,
          this.pickLine('left', BODY_LEFT_LINES),
          [],
          nowMs + QUIP_DURATION_MS,
        ),
      };
    }
    if (zone === 'body-right') {
      this.recordInteraction(1, 0);
      return {
        steps: [{ state: 'running-left', durationMs: 950 }],
        effect: { x, y, kind: 'note', bounce: 0.95 },
        sound: 'dodge',
        bubble: this.makeBubble(
          `${context.petName} · 躲闪`,
          this.pickLine('right', BODY_RIGHT_LINES),
          [],
          nowMs + QUIP_DURATION_MS,
        ),
      };
    }
    if (zone === 'feet') {
      this.recordInteraction(1, 0);
      return {
        steps: [{ state: 'jumping' }],
        effect: { x, y, kind: 'star', bounce: 1.2 },
        sound: 'tickle_feet',
        bubble: this.makeBubble(
          `${context.petName} · 好痒呀`,
          this.pickLine('feet', FEET_LINES),
          [],
          nowMs + QUIP_DURATION_MS,
        ),
      };
    }

    this.recordInteraction(1, 0);
    return {
      steps: [{ state: 'waving' }],
      effect: { x, y, kind: 'sparkle', bounce: 1.0 },
      sound: 'poke_body',
      bubble: this.makeBubble(
        `${context.petName} · 轻拍回应`,
        this.pickLine('center', BODY_CENTER_LINES),
        [],
        nowMs + QUIP_DURATION_MS,
      ),
    };
  }

  handleAction(action: InteractionBubbleAction, nowMs: number, context: InteractionContext): InteractionOutcome {
    if (action === 'pet_notifications') {
      return { openNotifications: true, closeBubble: true };
    }
    if (action === 'pet_feed') {
      this.recordInteraction(2, 1);
      return {
        steps: [{ state: 'jumping' }],
        effect: { x: 0.5, y: 0.68, kind: 'snack', bounce: 1.15, count: 6 },
        sound: 'feed',
        bubble: this.makePanelBubble(context.petName, this.pickLine('feed', FEED_LINES), nowMs),
      };
    }
    this.recordInteraction(2, 0);
    return {
      steps: [
        { state: 'running-right', durationMs: 700 },
        { state: 'running-left', durationMs: 700 },
        { state: 'jumping' },
      ],
      effect: { x: 0.5, y: 0.66, kind: 'note', bounce: 1.1, count: 6 },
      sound: 'play',
      bubble: this.makePanelBubble(context.petName, this.pickLine('play', PLAY_LINES), nowMs),
    };
  }

  handleDragEnd(nowMs: number, context: InteractionContext): InteractionOutcome | undefined {
    if (nowMs - this.lastDragReactAtMs < DRAG_REACT_COOLDOWN_MS) return undefined;
    this.lastDragReactAtMs = nowMs;
    return {
      steps: [{ state: 'jumping' }],
      effect: { x: 0.5, y: 0.72, kind: 'sparkle', bounce: 0.9, count: 4 },
      sound: 'drag_drop',
      bubble: this.makeBubble(
        `${context.petName} · 安顿好啦`,
        this.pickLine('drag', DRAG_LINES),
        [],
        nowMs + 2400,
      ),
    };
  }

  private recordInteraction(affinityDelta: number, feedDelta: number): void {
    this.stats = {
      affinity: this.stats.affinity + affinityDelta,
      interactions: this.stats.interactions + 1,
      feeds: this.stats.feeds + feedDelta,
    };
    saveStats(this.stats);
  }

  private pickLine(bucket: string, lines: string[]): string {
    const index = this.lineCounters.get(bucket) ?? 0;
    this.lineCounters.set(bucket, index + 1);
    return lines[index % lines.length];
  }

  private makePanelBubble(petName: string, detail: string, nowMs: number): InteractionBubble {
    const tier = affinityTier(this.stats.affinity);
    return this.makeBubble(
      `${petName} · ${tier} ♥ ${this.stats.affinity}`,
      detail,
      ['pet_feed', 'pet_play', 'pet_notifications'],
      nowMs + PANEL_DURATION_MS,
    );
  }

  private makeBubble(
    title: string,
    detail: string,
    actions: InteractionBubbleAction[],
    expiresAtMs: number,
  ): InteractionBubble {
    return {
      id: `pet-interaction:${++this.bubbleSeq}`,
      title,
      detail,
      actions,
      expiresAtMs,
    };
  }
}
