import { expect, it } from 'vitest';
import {
  affinityTier,
  classifyHitZone,
  PetInteractionController,
} from './pet-interactions';
import type { PetSnapshot } from '../types';

it('按桌宠底部舞台坐标准确区分摸头、左右侧、身体中部与脚底区域', () => {
  // 窗口 240x440，桌宠舞台 x: [40/240, 200/240], y: [260/440, 440/440]
  expect(classifyHitZone(0.5, 0.68)).toBe('head');
  expect(classifyHitZone(0.25, 0.8)).toBe('body-left');
  expect(classifyHitZone(0.5, 0.8)).toBe('body-center');
  expect(classifyHitZone(0.75, 0.8)).toBe('body-right');
  expect(classifyHitZone(0.5, 0.94)).toBe('feet');
});

it('亲密度等级随互动稳步进阶', () => {
  expect(affinityTier(0)).toBe('初来乍到');
  expect(affinityTier(10)).toBe('渐渐熟络');
  expect(affinityTier(30)).toBe('默契搭档');
  expect(affinityTier(70)).toBe('形影不离');
});

it('单击不同部位触发对应的动作、粒子特效与回应文本', () => {
  const controller = new PetInteractionController({ affinity: 0, interactions: 0, feeds: 0 });
  const ctx = { petName: '夏乌拉' };

  const head = controller.handleClick({ x: 0.5, y: 0.68 }, 1000, ctx);
  expect(head.steps).toEqual([{ state: 'waving' }]);
  expect(head.effect?.kind).toBe('heart');
  expect(head.bubble?.title).toContain('摸摸头');
  expect(head.bubble?.actions).toEqual([]);

  const left = controller.handleClick({ x: 0.25, y: 0.8 }, 2000, ctx);
  expect(left.steps).toEqual([{ state: 'running-right', durationMs: 950 }]);
  expect(left.effect?.kind).toBe('note');

  const right = controller.handleClick({ x: 0.75, y: 0.8 }, 3000, ctx);
  expect(right.steps).toEqual([{ state: 'running-left', durationMs: 950 }]);

  const feet = controller.handleClick({ x: 0.5, y: 0.94 }, 4000, ctx);
  expect(feet.steps).toEqual([{ state: 'jumping' }]);
  expect(feet.effect?.kind).toBe('star');

  const center = controller.handleClick({ x: 0.5, y: 0.8 }, 5000, ctx);
  expect(center.steps).toEqual([{ state: 'waving' }]);
  expect(center.effect?.kind).toBe('sparkle');
  expect(controller.getStats().interactions).toBe(5);
  expect(controller.getStats().affinity).toBe(5);
});

it('双击打开快捷互动面板，三连击触发应援连招，连续猛戳触发眩晕彩蛋与冷却', () => {
  const controller = new PetInteractionController({ affinity: 9, interactions: 4, feeds: 1 });
  const runningSnapshot: PetSnapshot = {
    revision: 1,
    sessions: [
      {
        key: { source: 'codex', installationId: 'i1', sessionId: 's1' },
        status: 'running',
        pending: [],
        unread: false,
        lastEventAtMs: 100,
      },
    ],
    notifications: [],
  };
  const ctx = { petName: '夏乌拉', snapshot: runningSnapshot };

  // 第 1 次点击
  controller.handleClick({ x: 0.5, y: 0.7, clickCount: 1 }, 1000, ctx);
  // 200ms 内第 2 次点击 -> 双击打开快捷互动面板
  const dbl = controller.handleClick({ x: 0.5, y: 0.7, clickCount: 2 }, 1200, ctx);
  expect(dbl.steps).toEqual([{ state: 'jumping' }]);
  expect(dbl.bubble?.actions).toEqual(['pet_feed', 'pet_play', 'pet_notifications']);
  expect(dbl.bubble?.title).toContain('渐渐熟络');
  expect(dbl.bubble?.detail).toContain('1 个会话运行中');

  // 紧接着第 3 次点击 -> 三连击连招
  const triple = controller.handleClick({ x: 0.5, y: 0.7, clickCount: 3 }, 1400, ctx);
  expect(triple.steps).toEqual([{ state: 'running-right', durationMs: 650 }, { state: 'jumping' }]);
  expect(triple.bubble?.title).toContain('三连击');

  // 短时间内连续戳到第 6 次 -> 触发眩晕彩蛋
  controller.handleClick({ x: 0.5, y: 0.7, clickCount: 1 }, 1900, ctx);
  controller.handleClick({ x: 0.5, y: 0.7, clickCount: 1 }, 2400, ctx);
  const dizzy = controller.handleClick({ x: 0.5, y: 0.7, clickCount: 1 }, 2900, ctx);
  expect(dizzy.steps).toEqual([{ state: 'failed' }]);
  expect(dizzy.effect?.kind).toBe('dizzy');
  expect(dizzy.bubble?.title).toContain('转晕啦');

  // 眩晕冷却期内点击不重复触发 failed
  const cooling = controller.handleClick({ x: 0.5, y: 0.7, clickCount: 1 }, 3500, ctx);
  expect(cooling.steps).toBeUndefined();
  expect(cooling.bubble?.title).toContain('缓一缓');
});

it('支持 Alt 快捷投喂、Shift 动作轮播、面板按钮动作及拖拽放置反馈', () => {
  const controller = new PetInteractionController({ affinity: 0, interactions: 0, feeds: 0 });
  const ctx = { petName: '夏乌拉' };

  const altFeed = controller.handleClick({ x: 0.5, y: 0.6, altKey: true }, 1000, ctx);
  expect(altFeed.steps).toEqual([{ state: 'jumping' }]);
  expect(altFeed.effect?.kind).toBe('snack');
  expect(controller.getStats()).toEqual({ affinity: 2, interactions: 1, feeds: 1 });

  const shift1 = controller.handleClick({ x: 0.5, y: 0.6, shiftKey: true }, 1500, ctx);
  const shift2 = controller.handleClick({ x: 0.5, y: 0.6, shiftKey: true }, 2000, ctx);
  expect(shift1.steps?.[0].state).toBe('jumping');
  expect(shift2.steps?.[0].state).toBe('waving');
  expect(shift2.bubble?.detail).toContain('招手');

  const feedBtn = controller.handleAction('pet_feed', 2500, ctx);
  expect(feedBtn.steps).toEqual([{ state: 'jumping' }]);
  expect(feedBtn.bubble?.actions).toEqual(['pet_feed', 'pet_play', 'pet_notifications']);
  expect(controller.getStats().feeds).toBe(2);

  const playBtn = controller.handleAction('pet_play', 3000, ctx);
  expect(playBtn.steps).toEqual([
    { state: 'running-right', durationMs: 700 },
    { state: 'running-left', durationMs: 700 },
    { state: 'jumping' },
  ]);

  const notifBtn = controller.handleAction('pet_notifications', 3500, ctx);
  expect(notifBtn).toEqual({ openNotifications: true, closeBubble: true });

  // 拖拽落地反馈带冷却保护
  const drag1 = controller.handleDragEnd(4000, ctx);
  expect(drag1?.steps).toEqual([{ state: 'jumping' }]);
  expect(controller.handleDragEnd(6000, ctx)).toBeUndefined();
  expect(controller.handleDragEnd(12500, ctx)?.steps).toEqual([{ state: 'jumping' }]);
});
