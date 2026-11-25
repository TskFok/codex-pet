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
    if (this.localName === 'canvas') return rect(0, 0, 240, 320);
    if (this.localName === 'aside') return rect(4, 54, 232, 80);
    if (this.dataset.action === 'close') return rect(212, 60, 18, 18);
    return this.dataset.action === 'session' ? rect(12, 102, 66, 20) : rect(82, 102, 56, 20);
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
  canvas.width = 480;
  canvas.height = 640;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 480, height: 640, runs: [] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons).toEqual([
      { action: 'close', rect: { x: 424, y: 120, width: 36, height: 36 } },
      { action: 'session', rect: { x: 24, y: 204, width: 132, height: 40 } },
      { action: 'copy', rect: { x: 164, y: 204, width: 112, height: 40 } },
    ]));
    expect(poses.mock.lastCall?.[0].bubbleInput?.backdrop).toEqual({
      x: 8,
      y: 108,
      width: 464,
      height: 160,
      radius: 28,
    });
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
  let cssWidth = 360, cssHeight = 480;
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, cssWidth, cssHeight);
    if (this.localName === 'aside') return rect(4, 50, cssWidth - 8, 80);
    if (this.dataset.action === 'close') return rect(cssWidth - 26, 56, 18, 18);
    // 故意让 copy 按钮稍微越出右边界，模拟窄窗口溢出
    return this.dataset.action === 'session' ? rect(12, 98, 60, 20) : rect(cssWidth - 30, 98, 40, 20);
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
  canvas.width = 720;
  canvas.height = 960;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 720, height: 960, runs: [] }, { x: 0, y: 0, width: 360, height: 480 });
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.length).toBe(3));
    // 右边界越界按钮必须被裁剪在 mask.width 以内
    const initialCopy = poses.mock.lastCall![0].bubbleInput!.buttons[2];
    expect(initialCopy.rect.x + initialCopy.rect.width).toBeLessThanOrEqual(720);

    // 模拟从 150% (360x480 @2x = 720x960) 缩小为 100% (240x320 @1x = 240x320)，且 ResizeObserver 未在新 mask 后再次触发
    cssWidth = 240;
    cssHeight = 320;
    canvas.width = 240;
    canvas.height = 320;
    handle.pose({ width: 240, height: 320, runs: [] }, { x: 0, y: 0, width: 240, height: 320 });
    const shrunk = poses.mock.lastCall![0];
    expect(shrunk.mask.width).toBe(240);
    expect(shrunk.bubbleInput?.buttons[1]).toEqual({ action: 'session', rect: { x: 12, y: 98, width: 60, height: 20 } });
    for (const button of shrunk.bubbleInput!.buttons) {
      expect(button.rect.x).toBeGreaterThanOrEqual(0);
      expect(button.rect.y).toBeGreaterThanOrEqual(0);
      expect(button.rect.x + button.rect.width).toBeLessThanOrEqual(240);
      expect(button.rect.y + button.rect.height).toBeLessThanOrEqual(320);
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
    if (this.localName === 'canvas') return rect(0, 0, 240, 320);
    if (this.localName === 'aside') return rect(4, 54, 232, 80);
    if (this.dataset.action === 'close') return rect(212, 60, 18, 18);
    if (this.dataset.action === 'session') return rect(12, 102, 60, 20);
    if (this.dataset.action === 'application') return rect(76, 102, 60, 20);
    return rect(140, 102, 56, 20);
  });
  const snapshot = structuredClone(fixture.expected) as PetSnapshot;
  const prefs: Settings = {
    revision: 3, fps: 30, sound: false, autostart: false, mutedSources: [], petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
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
  canvas.height = 640;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 480, height: 640, runs: [] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map(b => b.action)).toEqual([
      'close', 'session', 'copy',
    ]));
    const stampBefore = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(stampBefore, 'session');
    await vi.waitFor(() => expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map(b => b.action)).toEqual([
      'close', 'session', 'application', 'copy',
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

it('点击关闭按钮隐藏提示框并释放气泡热区，无关刷新不重新弹出，新提醒到来时再次显示', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 320);
    if (this.localName === 'aside') return rect(4, 54, 232, 80);
    if (this.dataset.action === 'close') return rect(212, 60, 18, 18);
    return this.dataset.action === 'session' ? rect(12, 102, 60, 20) : rect(76, 102, 56, 20);
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
  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 320;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const bubble = document.querySelector('aside')!;
  try {
    handle.pose({ width: 240, height: 320, runs: [[200, 80, 160]] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(bubble.hidden).toBe(false));
    const closeStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(closeStamp, 'close');
    expect(bubble.hidden).toBe(true);
    expect(bubble.textContent).toBe('');
    expect(poses.mock.lastCall?.[0].bubbleInput).toBeUndefined();
    expect(poses.mock.lastCall?.[0].mask.runs).toEqual([[200, 80, 160]]);

    // 同一提醒收到无关快照更新时保持关闭
    const unrelated = structuredClone(snapshot);
    unrelated.revision += 1;
    update(unrelated);
    expect(bubble.hidden).toBe(true);

    // 收到新的待处理请求时重新弹出提示框，且直接点击关闭按钮也可关闭
    const fresh = structuredClone(unrelated);
    fresh.revision += 1;
    fresh.sessions[0].pending = [{ id: 'req-new', reason: 'answer', summary: '请确认下一步' }];
    update(fresh);
    expect(bubble.hidden).toBe(false);
    expect(bubble.textContent).toContain('请确认下一步');
    const closeButton = bubble.querySelector<HTMLButtonElement>('button[data-action="close"]')!;
    closeButton.click();
    expect(bubble.hidden).toBe(true);
    expect(poses.mock.lastCall?.[0].bubbleInput).toBeUndefined();
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('宠物加载失败提示框也包含关闭按钮且可关闭', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 320);
    if (this.localName === 'aside') return rect(4, 70, 232, 64);
    return rect(212, 76, 18, 18);
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
  const prefs: Settings = {
    revision: 1, fps: 30, sound: false, autostart: false, mutedSources: [], petId: 'missing-pet',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshot').mockResolvedValue({ revision: 1, sessions: [], notifications: [] });
  vi.spyOn(api, 'asset').mockRejectedValue(new Error('not found'));
  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 320;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn(), mount: vi.fn() };
  const mountDefault = vi.fn().mockResolvedValue(undefined);
  const handle = bindReminder(canvas, { renderer, mountDefault } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const bubble = document.querySelector('aside')!;
  try {
    handle.pose({ width: 240, height: 320, runs: [] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(bubble.hidden).toBe(false));
    expect(bubble.textContent).toContain('无法加载所选桌宠');
    expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map(b => b.action)).toEqual(['close']);
    deliverBubbleAction(poses.mock.lastCall![0].bubbleInput!.stamp, 'close');
    expect(bubble.hidden).toBe(true);
    expect(poses.mock.lastCall?.[0].bubbleInput).toBeUndefined();
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('任务完成动作播完后自动切回待机或运行中，无关刷新不重播，新完成或失败再触发一次', async () => {
  const { document } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  let update!: (value: PetSnapshot) => void;
  const keyA = { source: 'codex' as const, installationId: 'i1', sessionId: 'a' };
  const keyB = { source: 'codex' as const, installationId: 'i1', sessionId: 'b' };
  const initial: PetSnapshot = {
    revision: 1,
    sessions: [
      { key: keyA, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 },
    ],
    notifications: [
      {
        id: 'end-1',
        key: keyA,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        occurredAtMs: 100,
        read: false,
      },
    ],
  };
  const prefs: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    mutedSources: [],
    petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockImplementation(async (receive) => {
    update = receive;
    return () => {};
  });
  vi.spyOn(api, 'snapshot').mockResolvedValue(initial);
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(
    document.querySelector('canvas')!,
    { renderer } as unknown as ReturnType<typeof mountPet>,
    vi.fn(),
    vi.fn(),
  );
  try {
    await vi.waitFor(() => expect(renderer.setState).toHaveBeenCalledWith('waving', expect.any(Number)));

    // 单次 waving 播完后回到 idle
    handle.animationEnded('waving', 700);
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', 700);

    // 同一未读完成通知收到无关快照更新时保持 idle，不重新触发 waving
    update({ ...initial, revision: 2 });
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', expect.any(Number));

    // 另一会话开始运行后切到 running，而不是继续卡在 waving
    const runningB: PetSnapshot = {
      ...initial,
      revision: 3,
      sessions: [
        ...initial.sessions,
        { key: keyB, turnId: 't2', status: 'running', pending: [], unread: false, lastEventAtMs: 200 },
      ],
    };
    update(runningB);
    expect(renderer.setState).toHaveBeenLastCalledWith('running', expect.any(Number));

    // 会话 B 完成时触发新的 waving，并在播完后回到 idle
    const endedB: PetSnapshot = {
      revision: 4,
      sessions: [
        initial.sessions[0],
        { key: keyB, turnId: 't2', status: 'ended', pending: [], unread: true, lastEventAtMs: 300 },
      ],
      notifications: [
        ...initial.notifications,
        {
          id: 'end-2',
          key: keyB,
          turnId: 't2',
          kind: { type: 'turn_ended', outcome: 'completed' },
          occurredAtMs: 300,
          read: false,
        },
      ],
    };
    update(endedB);
    expect(renderer.setState).toHaveBeenLastCalledWith('waving', expect.any(Number));
    handle.animationEnded('waving', 1400);
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', 1400);

    // 失败动作播完后也回到 idle，随后标为已读不重播 failed
    const failedB: PetSnapshot = {
      revision: 5,
      sessions: [
        { key: keyB, turnId: 't3', status: 'failed', pending: [], unread: true, lastEventAtMs: 400 },
      ],
      notifications: [
        {
          id: 'fail-3',
          key: keyB,
          turnId: 't3',
          kind: { type: 'turn_ended', outcome: 'failed' },
          occurredAtMs: 400,
          read: false,
        },
      ],
    };
    update(failedB);
    expect(renderer.setState).toHaveBeenLastCalledWith('failed', expect.any(Number));
    handle.animationEnded('failed', 2620);
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', 2620);

    update({
      ...failedB,
      revision: 6,
      sessions: [{ ...failedB.sessions[0], unread: false }],
      notifications: [{ ...failedB.notifications[0], read: true }],
    });
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', expect.any(Number));
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('支持点击桌宠触发部位互动、双击快捷面板按钮及真实任务提醒优先抢占', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 320);
    if (this.localName === 'aside') return rect(4, 54, 232, 80);
    if (this.dataset.action === 'close') return rect(212, 60, 18, 18);
    if (this.dataset.action === 'pet_feed') return rect(12, 102, 52, 20);
    if (this.dataset.action === 'pet_play') return rect(68, 102, 52, 20);
    if (this.dataset.action === 'pet_notifications') return rect(124, 102, 52, 20);
    return rect(12, 102, 60, 20);
  });
  let update!: (value: PetSnapshot) => void;
  const idleSnapshot: PetSnapshot = { revision: 1, sessions: [], notifications: [] };
  const prefs: Settings = {
    revision: 1, fps: 30, sound: false, autostart: false, mutedSources: [], petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockImplementation(async (receive) => { update = receive; return () => {}; });
  vi.spyOn(api, 'snapshot').mockResolvedValue(idleSnapshot);
  const openNotifications = vi.spyOn(api, 'openNotifications').mockResolvedValue(undefined);
  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 320;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn(), triggerInteractionEffect: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const bubble = document.querySelector('aside')!;
  const deliverEvent = (name: string, payload: unknown) => {
    const call = [...vi.mocked(listen).mock.calls].reverse().find(([eventName]) => eventName === name);
    if (!call) throw new Error(`缺少 ${name} 事件监听`);
    call[1]({ event: name, id: 1, payload });
  };
  try {
    handle.pose({ width: 240, height: 320, runs: [[200, 80, 160]] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(renderer.setState).toHaveBeenCalledWith('idle', expect.any(Number)));

    // 1. 通过原生 pet:click 摸头：触发招手、爱心特效与回应气泡，播完恢复 idle
    deliverEvent('pet:click', { x: 0.5, y: 0.5, clickCount: 1 });
    expect(renderer.setState).toHaveBeenLastCalledWith('waving', expect.any(Number), true);
    expect(renderer.triggerInteractionEffect).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'heart' }),
      expect.any(Number),
    );
    expect(bubble.hidden).toBe(false);
    expect(bubble.textContent).toContain('摸摸头');
    handle.animationEnded('waving', 800);
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', expect.any(Number));

    // 2. 双击打开快捷互动面板，包含投喂/玩耍/通知按钮并同步原生命中区域
    handle.click({ x: 0.5, y: 0.72, clickCount: 2 });
    expect(renderer.setState).toHaveBeenLastCalledWith('jumping', expect.any(Number), true);
    expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map((b) => b.action)).toEqual([
      'close',
      'pet_feed',
      'pet_play',
      'pet_notifications',
    ]);

    // 3. 通过原生气泡点击触发“玩耍”连招与“通知”打开
    const panelStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(panelStamp, 'pet_play');
    expect(renderer.setState).toHaveBeenLastCalledWith('running-right', expect.any(Number), true);
    const updatedStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(updatedStamp, 'pet_notifications');
    expect(openNotifications).toHaveBeenCalledTimes(1);
    expect(bubble.hidden).toBe(true);

    // 4. 真实任务提醒到达时立刻抢占互动气泡，且提醒存在时点击桌宠不覆盖真实提醒
    handle.click({ x: 0.5, y: 0.5, clickCount: 1 });
    expect(bubble.hidden).toBe(false);
    const urgent = structuredClone(fixture.expected) as PetSnapshot;
    urgent.revision = 10;
    update(urgent);
    expect(renderer.setState).toHaveBeenLastCalledWith('review', expect.any(Number));
    expect(bubble.textContent).toContain('待审批');

    renderer.triggerInteractionEffect.mockClear();
    handle.click({ x: 0.5, y: 0.72, clickCount: 2 });
    expect(renderer.triggerInteractionEffect).toHaveBeenCalledTimes(1);
    expect(bubble.textContent).toContain('待审批');
    expect(bubble.textContent).not.toContain('投喂');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
