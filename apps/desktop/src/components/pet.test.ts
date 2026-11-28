import { expect, it, vi } from 'vitest';
import whaleMaidManifest from '../assets/deepseek-whale-maid.json';
import defaultManifest from '../assets/yueying.json';
import {
  bindPetInput,
  PET_STAGE_HEIGHT,
  PET_STAGE_WIDTH,
  PET_WINDOW_HEIGHT,
  PET_WINDOW_WIDTH,
  PetVisibility,
  petViewport,
  type PlaybackSignal,
} from './pet';

it('DeepSeek 娘清单保留九行动作与两行共十六个方向矩形', () => {
  expect(whaleMaidManifest).toMatchObject({
    version: 1,
    id: 'deepseek-whale-maid',
    name: 'DeepSeek娘',
    atlas: { path: 'atlas.webp', width: 1536, height: 2288, cellWidth: 192, cellHeight: 208 },
  });
  expect(Object.keys(whaleMaidManifest.animations)).toHaveLength(9);
  expect(whaleMaidManifest.animations).toEqual(defaultManifest.animations);
  expect(whaleMaidManifest.lookDirections).toEqual(Array.from({ length: 16 }, (_, index) => ({
    x: (index % 8) * 192,
    y: 1872 + Math.floor(index / 8) * 208,
    width: 192,
    height: 208,
  })));
});

it('拒绝晚到的旧可见状态，跨过休眠恢复事件也会重开动作并切断测量连续性', () => {
  const setVisible = vi.fn();
  const breakContinuity = vi.fn();
  const lifecycle = new PetVisibility(setVisible, breakContinuity);
  const signal: PlaybackSignal = { revision: 0, visible: true, suspended: false };
  lifecycle.update(signal, 0);
  lifecycle.update({ ...signal, revision: 2 }, 3000);
  lifecycle.update({ ...signal, revision: 1, suspended: true }, 3001);
  lifecycle.setDocumentVisible(false, 3002);
  lifecycle.update({ ...signal, revision: 3, visible: false }, 3003);
  lifecycle.setDocumentVisible(true, 4100);
  lifecycle.update({ ...signal, revision: 4 }, 5000);
  expect(setVisible.mock.calls).toEqual([
    [true, 0], [false, 3000], [true, 3000], [false, 3002], [true, 5000],
  ]);
  expect(breakContinuity).toHaveBeenCalledTimes(2);
});

it('桌宠视口始终位于窗口底部居中，上方为提示框预留无遮挡空间', () => {
  expect(petViewport(PET_WINDOW_WIDTH, PET_WINDOW_HEIGHT)).toEqual({
    x: 40,
    y: PET_WINDOW_HEIGHT - PET_STAGE_HEIGHT,
    width: PET_STAGE_WIDTH,
    height: PET_STAGE_HEIGHT,
  });
  expect(petViewport(360, 660)).toEqual({
    x: 60,
    y: 390,
    width: 240,
    height: 270,
  });
});

it('Windows 左键拖动触发原生拖动，单击与双击触发互动，右键唤起原生菜单', () => {
  const target = new EventTarget();
  const canvas = Object.assign(new EventTarget(), {
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 240, height: 320 }),
  });
  const invoke = vi.fn().mockResolvedValue(undefined);
  const onPetClick = vi.fn();
  const dispose = bindPetInput(target, canvas, 'windows', invoke, onPetClick);

  // 轻微抖动后抬起：判定为单击互动，不启动窗口拖动
  canvas.dispatchEvent(
    Object.assign(new Event('pointerdown'), { button: 0, pointerId: 1, clientX: 120, clientY: 160 }),
  );
  target.dispatchEvent(
    Object.assign(new Event('pointermove'), { pointerId: 1, clientX: 121, clientY: 161 }),
  );
  target.dispatchEvent(
    Object.assign(new Event('pointerup'), { pointerId: 1, clientX: 121, clientY: 161 }),
  );
  expect(invoke).not.toHaveBeenCalled();
  expect(onPetClick).toHaveBeenCalledTimes(1);
  expect(onPetClick).toHaveBeenLastCalledWith({
    x: 121 / 240,
    y: 161 / 320,
    clickCount: 1,
    altKey: false,
    shiftKey: false,
  });

  // 紧接着再次点击：判定为双击
  canvas.dispatchEvent(
    Object.assign(new Event('pointerdown'), { button: 0, pointerId: 2, clientX: 122, clientY: 162, shiftKey: true }),
  );
  target.dispatchEvent(
    Object.assign(new Event('pointerup'), { pointerId: 2, clientX: 122, clientY: 162 }),
  );
  expect(onPetClick).toHaveBeenCalledTimes(2);
  expect(onPetClick).toHaveBeenLastCalledWith({
    x: 122 / 240,
    y: 162 / 320,
    clickCount: 2,
    altKey: false,
    shiftKey: true,
  });

  // 按住移动超过阈值：触发原生窗口拖动，不再触发点击互动
  canvas.dispatchEvent(
    Object.assign(new Event('pointerdown'), { button: 0, pointerId: 3, clientX: 120, clientY: 200 }),
  );
  target.dispatchEvent(
    Object.assign(new Event('pointermove'), { pointerId: 3, clientX: 130, clientY: 205 }),
  );
  target.dispatchEvent(
    Object.assign(new Event('pointerup'), { pointerId: 3, clientX: 130, clientY: 205 }),
  );
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke).toHaveBeenLastCalledWith('pointer_input');
  expect(onPetClick).toHaveBeenCalledTimes(2);

  // 右键按下不触发拖动，contextmenu 唤起原生菜单
  canvas.dispatchEvent(Object.assign(new Event('pointerdown'), { button: 2, pointerId: 4 }));
  const menuEvent = new Event('contextmenu', { cancelable: true });
  target.dispatchEvent(menuEvent);
  expect(menuEvent.defaultPrevented).toBe(true);
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenLastCalledWith('show_pet_menu');

  dispose();
  const afterDispose = new Event('contextmenu', { cancelable: true });
  target.dispatchEvent(afterDispose);
  expect(afterDispose.defaultPrevented).toBe(false);
  expect(invoke).toHaveBeenCalledTimes(2);
});

it('macOS 仅拦截默认右键菜单，不从 WebView 重复触发原生菜单命令', () => {
  const target = new EventTarget();
  const canvas = new EventTarget();
  const invoke = vi.fn().mockResolvedValue(undefined);
  const dispose = bindPetInput(target, canvas, 'macos', invoke);

  const menuEvent = new Event('contextmenu', { cancelable: true });
  target.dispatchEvent(menuEvent);
  expect(menuEvent.defaultPrevented).toBe(true);
  expect(invoke).not.toHaveBeenCalled();

  dispose();
});
