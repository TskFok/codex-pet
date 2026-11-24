import { parseHTML } from 'linkedom';
import { expect, it, vi } from 'vitest';
import { api } from '../api';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { bindReminder } from './pet-bindings';
import type { mountPet } from './pet';
import type { PetSnapshot, Settings } from '../types';
import fixture from '../../../../tests/fixtures/events/child-folding.json';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

function deliverBubbleAction(stamp: number, action: string) {
  const handler = vi.mocked(listen).mock.lastCall?.[1];
  if (!handler) throw new Error('缺少气泡输入监听');
  handler({ event: 'pet:bubble-action', id: 1, payload: { stamp, action } });
}

it('设置读取失败或晚到时，连续快照也不泄露已静音的项目或摘要', async () => {
  const { document } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
  vi.spyOn(console, 'error').mockImplementation(() => {});
  let fail!: (e: Error) => void, receive!: (v: Settings) => void, update!: (v: PetSnapshot) => void;
  vi.spyOn(api, 'settings').mockImplementation(
    () =>
      new Promise((_, reject) => {
        fail = reject;
      }),
  );
  vi.spyOn(api, 'settingsEvents').mockImplementation(async (cb) => {
    receive = cb;
    return () => {};
  });
  vi.spyOn(api, 'snapshots').mockImplementation(async (cb) => {
    update = cb;
    return () => {};
  });
  const snapshot = structuredClone(fixture.expected) as PetSnapshot;
  snapshot.sessions[0].project = { name: '私有项目' };
  vi.spyOn(api, 'snapshot').mockResolvedValue(snapshot);
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(
    document.querySelector('canvas')!,
    { renderer } as unknown as ReturnType<typeof mountPet>,
    vi.fn(),
    vi.fn(),
  );
  const bubble = document.querySelector('aside')!;
  const prefs: Settings = {
    revision: 3,
    fps: 30,
    sound: true,
    autostart: false,
    mutedSources: ['codex'],
    petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  try {
    await vi.waitFor(() => expect(renderer.setState).toHaveBeenCalledWith('review', expect.any(Number)));
    expect(bubble.hidden).toBe(true);
    expect(bubble.textContent).toBe('');
    fail(new Error('读取失败'));
    await Promise.resolve();
    update({ ...snapshot, revision: snapshot.revision + 1 });
    expect(bubble.textContent).toBe('');
    receive(prefs);
    expect(bubble.hidden).toBe(true);
    expect(bubble.textContent).toBe('');
    expect(invoke).not.toHaveBeenCalled();
    expect(renderer.setState).toHaveBeenLastCalledWith('review', expect.any(Number));
    receive({ ...prefs, revision: 2, mutedSources: [] });
    expect(bubble.hidden).toBe(true);
    receive({ ...prefs, revision: 4, mutedSources: [] });
    expect(bubble.hidden).toBe(false);
    expect(bubble.textContent).toContain('私有项目');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('按气泡真实按钮位置发布命中，并忽略上一会话晚到的原生点击', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 160, 180);
    if (this.localName === 'aside') return rect(2, 2, 156, 60);
    return this.dataset.action === 'session' ? rect(8, 34, 66, 20) : rect(78, 34, 56, 20);
  });
  let update!: (value: PetSnapshot) => void;
  const snapshot = structuredClone(fixture.expected) as PetSnapshot;
  const prefs: Settings = {
    revision: 3, fps: 30, sound: false, autostart: false, mutedSources: [], petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockImplementation(async (receive) => { update = receive; return () => {}; });
  vi.spyOn(api, 'snapshot').mockResolvedValue(snapshot);
  const navigate = vi.spyOn(api, 'openSession').mockResolvedValue({ status: 'requested' });
  const canvas = document.querySelector('canvas')!;
  canvas.width = 320;
  canvas.height = 360;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 320, height: 360, runs: [] }, { x: 0, y: 0, width: 160, height: 180 });
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons[0]).toEqual(
      { action: 'session', rect: { x: 16, y: 68, width: 132, height: 40 } },
    ));
    const before = poses.mock.lastCall![0].bubbleInput!.stamp;
    const changed = structuredClone(snapshot);
    changed.revision++;
    changed.sessions[0].key.sessionId = '新会话';
    update(changed);
    const after = poses.mock.lastCall![0].bubbleInput!.stamp;
    expect(after).toBeGreaterThan(before);
    deliverBubbleAction(before, 'session');
    expect(navigate).not.toHaveBeenCalled();
    deliverBubbleAction(after, 'session');
    expect(navigate).toHaveBeenCalledWith(changed.sessions[0].key, 'session');
    await vi.waitFor(() => expect(document.querySelector('aside')?.textContent).toContain('已发送打开请求'));
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('缩放或 DPI 变化及气泡按钮越界时按新掩码重测并裁剪，避免命中错位或校验拒绝', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  let cssWidth = 240, cssHeight = 270;
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, cssWidth, cssHeight);
    if (this.localName === 'aside') return rect(2, 2, cssWidth - 4, 60);
    // 故意让 copy 按钮稍微越出右边界，模拟窄窗口溢出
    return this.dataset.action === 'session' ? rect(8, 34, 60, 20) : rect(cssWidth - 30, 34, 40, 20);
  });
  const snapshot = structuredClone(fixture.expected) as PetSnapshot;
  const prefs: Settings = {
    revision: 3, fps: 30, sound: false, autostart: false, mutedSources: [], petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1.5, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshot').mockResolvedValue(snapshot);
  const canvas = document.querySelector('canvas')!;
  canvas.width = 480;
  canvas.height = 540;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 480, height: 540, runs: [] }, { x: 0, y: 0, width: 240, height: 270 });
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.length).toBe(2));
    // 右边界越界按钮必须被裁剪在 mask.width 以内
    const initialCopy = poses.mock.lastCall![0].bubbleInput!.buttons[1];
    expect(initialCopy.rect.x + initialCopy.rect.width).toBeLessThanOrEqual(480);

    // 模拟从 150% (240x270 @2x = 480x540) 缩小为 100% (160x180 @1x = 160x180)，且 ResizeObserver 未在新 mask 后再次触发
    cssWidth = 160;
    cssHeight = 180;
    canvas.width = 160;
    canvas.height = 180;
    handle.pose({ width: 160, height: 180, runs: [] }, { x: 0, y: 0, width: 160, height: 180 });
    const shrunk = poses.mock.lastCall![0];
    expect(shrunk.mask.width).toBe(160);
    expect(shrunk.bubbleInput?.buttons[0]).toEqual({ action: 'session', rect: { x: 8, y: 34, width: 60, height: 20 } });
    for (const button of shrunk.bubbleInput!.buttons) {
      expect(button.rect.x).toBeGreaterThanOrEqual(0);
      expect(button.rect.y).toBeGreaterThanOrEqual(0);
      expect(button.rect.x + button.rect.width).toBeLessThanOrEqual(160);
      expect(button.rect.y + button.rect.height).toBeLessThanOrEqual(180);
    }
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('气泡外框尺寸不变时降级显示打开应用按钮，仍立即同步原生按钮区域与修订号', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 270);
    if (this.localName === 'aside') return rect(4, 4, 232, 64);
    if (this.dataset.action === 'session') return rect(8, 36, 60, 20);
    if (this.dataset.action === 'application') return rect(72, 36, 60, 20);
    return rect(136, 36, 56, 20);
  });
  const snapshot = structuredClone(fixture.expected) as PetSnapshot;
  const prefs: Settings = {
    revision: 3, fps: 30, sound: false, autostart: false, mutedSources: [], petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1.5, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshot').mockResolvedValue(snapshot);
  const navigate = vi.spyOn(api, 'openSession')
    .mockResolvedValueOnce({ status: 'application_only', opened: false })
    .mockResolvedValueOnce({ status: 'application_only', opened: true });
  const canvas = document.querySelector('canvas')!;
  canvas.width = 480;
  canvas.height = 540;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 480, height: 540, runs: [] }, { x: 0, y: 0, width: 240, height: 270 });
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map(b => b.action)).toEqual(['session', 'copy']));
    const stampBefore = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(stampBefore, 'session');
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map(b => b.action)).toEqual([
      'session', 'application', 'copy',
    ]));
    const stampAfter = poses.mock.lastCall![0].bubbleInput!.stamp;
    expect(stampAfter).toBeGreaterThan(stampBefore);
    deliverBubbleAction(stampAfter, 'application');
    expect(navigate).toHaveBeenLastCalledWith(snapshot.sessions[0].key, 'application');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
