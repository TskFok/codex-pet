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

function deliverBubbleAction(stamp: number, action: string, bubble = 0) {
  const handler = vi.mocked(listen).mock.lastCall?.[1];
  if (!handler) throw new Error('缺少气泡输入监听');
  handler({ event: 'pet:bubble-action', id: 1, payload: { stamp, bubble, action } });
}

function singleAttentionSnapshot(): PetSnapshot {
  const snap = structuredClone(fixture.expected) as PetSnapshot;
  snap.sessions = [snap.sessions[0]];
  snap.notifications = snap.notifications.filter((n) => n.key.sessionId === snap.sessions[0].key.sessionId);
  return snap;
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
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
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
    expect(stack.hidden).toBe(true);
    expect(stack.textContent).toBe('');
    fail(new Error('读取失败'));
    await Promise.resolve();
    update({ ...snapshot, revision: snapshot.revision + 1 });
    expect(stack.textContent).toBe('');
    receive(prefs);
    expect(stack.hidden).toBe(true);
    expect(stack.textContent).toBe('');
    expect(invoke).not.toHaveBeenCalled();
    expect(renderer.setState).toHaveBeenLastCalledWith('review', expect.any(Number));
    receive({ ...prefs, revision: 2, mutedSources: [] });
    expect(stack.hidden).toBe(true);
    receive({ ...prefs, revision: 4, mutedSources: [] });
    expect(stack.hidden).toBe(false);
    expect(stack.textContent).toContain('私有项目');
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
  const snapshot = singleAttentionSnapshot();
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
      { bubble: 0, action: 'close', rect: { x: 424, y: 120, width: 36, height: 36 } },
      { bubble: 0, action: 'session', rect: { x: 24, y: 204, width: 132, height: 40 } },
      { bubble: 0, action: 'copy', rect: { x: 164, y: 204, width: 112, height: 40 } },
    ]));
    expect(poses.mock.lastCall?.[0].bubbleInput?.backdrops).toEqual([
      {
        x: 8,
        y: 108,
        width: 464,
        height: 160,
        radius: 28,
      },
    ]);
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
  const snapshot = singleAttentionSnapshot();
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
    expect(shrunk.bubbleInput?.buttons[1]).toEqual({ bubble: 0, action: 'session', rect: { x: 12, y: 98, width: 60, height: 20 } });
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
  const snapshot = singleAttentionSnapshot();
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
  const snapshot = singleAttentionSnapshot();
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
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  try {
    handle.pose({ width: 240, height: 320, runs: [[200, 80, 160]] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(stack.hidden).toBe(false));
    const closeStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(closeStamp, 'close');
    expect(stack.hidden).toBe(true);
    expect(stack.textContent).toBe('');
    expect(poses.mock.lastCall?.[0].bubbleInput).toBeUndefined();
    expect(poses.mock.lastCall?.[0].mask.runs).toEqual([[200, 80, 160]]);

    // 同一提醒收到无关快照更新时保持关闭
    const unrelated = structuredClone(snapshot);
    unrelated.revision += 1;
    update(unrelated);
    expect(stack.hidden).toBe(true);

    // 收到新的待处理请求时重新弹出提示框，且直接点击关闭按钮也可关闭
    const fresh = structuredClone(unrelated);
    fresh.revision += 1;
    fresh.sessions[0].pending = [{ id: 'req-new', reason: 'answer', summary: '请确认下一步' }];
    update(fresh);
    expect(stack.hidden).toBe(false);
    expect(stack.textContent).toContain('请确认下一步');
    const closeButton = stack.querySelector<HTMLButtonElement>('button[data-action="close"]')!;
    closeButton.click();
    expect(stack.hidden).toBe(true);
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
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  try {
    handle.pose({ width: 240, height: 320, runs: [] }, { x: 0, y: 0, width: 240, height: 320 });
    await vi.waitFor(() => expect(stack.hidden).toBe(false));
    expect(stack.textContent).toContain('无法加载所选桌宠');
    expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map(b => b.action)).toEqual(['close']);
    deliverBubbleAction(poses.mock.lastCall![0].bubbleInput!.stamp, 'close');
    expect(stack.hidden).toBe(true);
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

it('支持点击桌宠触发部位互动、双击快捷面板按钮及真实任务提醒与互动气泡共存叠加', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      return rect(4, 174 - idx * 86, 232, 80);
    }
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = 174 - idx * 86;
    if (this.dataset.action === 'close') return rect(212, top + 6, 18, 18);
    if (this.dataset.action === 'pet_feed') return rect(12, top + 48, 52, 20);
    if (this.dataset.action === 'pet_play') return rect(68, top + 48, 52, 20);
    if (this.dataset.action === 'pet_notifications') return rect(124, top + 48, 52, 20);
    return rect(12, top + 48, 60, 20);
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
  canvas.height = 440;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn(), triggerInteractionEffect: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  const deliverEvent = (name: string, payload: unknown) => {
    const call = [...vi.mocked(listen).mock.calls].reverse().find(([eventName]) => eventName === name);
    if (!call) throw new Error(`缺少 ${name} 事件监听`);
    call[1]({ event: name, id: 1, payload });
  };
  try {
    handle.pose({ width: 240, height: 440, runs: [[300, 80, 160]] }, { x: 0, y: 0, width: 240, height: 440 });
    await vi.waitFor(() => expect(renderer.setState).toHaveBeenCalledWith('idle', expect.any(Number)));

    // 1. 通过原生 pet:click 摸头：触发招手、爱心特效与回应气泡，播完恢复 idle
    deliverEvent('pet:click', { x: 0.5, y: 0.68, clickCount: 1 });
    expect(renderer.setState).toHaveBeenLastCalledWith('waving', expect.any(Number), true);
    expect(renderer.triggerInteractionEffect).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'heart' }),
      expect.any(Number),
    );
    expect(stack.hidden).toBe(false);
    expect(stack.textContent).toContain('摸摸头');
    handle.animationEnded('waving', 800);
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', expect.any(Number));

    // 2. 双击打开快捷互动面板，包含投喂/玩耍/通知按钮并同步原生命中区域
    handle.click({ x: 0.5, y: 0.8, clickCount: 2 });
    expect(renderer.setState).toHaveBeenLastCalledWith('jumping', expect.any(Number), true);
    expect(poses.mock.lastCall?.[0].bubbleInput?.buttons.map((b) => b.action)).toEqual([
      'close',
      'pet_feed',
      'pet_play',
      'pet_notifications',
    ]);

    // 3. 通过原生气泡点击触发“玩耍”连招与“通知”打开
    const panelStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(panelStamp, 'pet_play', 0);
    expect(renderer.setState).toHaveBeenLastCalledWith('running-right', expect.any(Number), true);
    const updatedStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(updatedStamp, 'pet_notifications', 0);
    expect(openNotifications).toHaveBeenCalledTimes(1);
    expect(stack.hidden).toBe(true);

    // 4. 真实任务提醒到达时与互动气泡向上叠加共存，点击桌宠也不覆盖已有提醒
    handle.click({ x: 0.5, y: 0.68, clickCount: 1 });
    expect(stack.hidden).toBe(false);
    const urgent = singleAttentionSnapshot();
    urgent.revision = 10;
    update(urgent);
    expect(renderer.setState).toHaveBeenLastCalledWith('review', expect.any(Number));
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(2);
    expect(stack.textContent).toContain('待审批');

    renderer.triggerInteractionEffect.mockClear();
    handle.click({ x: 0.5, y: 0.8, clickCount: 2 });
    expect(renderer.triggerInteractionEffect).toHaveBeenCalledTimes(1);
    expect(stack.textContent).toContain('待审批');
    expect(stack.textContent).toContain('投喂');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('响应 DeepSeek 自主活动事件，用户点击可打断，紧急待审批抢占自主活动', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.localName === 'aside') return rect(4, 174, 232, 72);
    return rect(212, 180, 18, 18);
  });
  let update!: (value: PetSnapshot) => void;
  const idleSnapshot: PetSnapshot = { revision: 1, sessions: [], notifications: [] };
  const prefs: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    mutedSources: [],
    petId: 'yueying-cape-dancer',
    deepseekEnabled: true,
    deepseekModel: 'deepseek-chat',
    deepseekApiKeyConfigured: true,
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockImplementation(async (receive) => {
    update = receive;
    return () => {};
  });
  vi.spyOn(api, 'snapshot').mockResolvedValue(idleSnapshot);
  const interrupt = vi.spyOn(api, 'interruptPetActivity').mockResolvedValue(undefined);
  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const renderer = { setState: vi.fn(), setFps: vi.fn(), triggerInteractionEffect: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, vi.fn(), vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  const deliverEvent = (name: string, payload: unknown) => {
    const call = [...vi.mocked(listen).mock.calls].reverse().find(([eventName]) => eventName === name);
    if (!call) throw new Error(`缺少 ${name} 事件监听`);
    call[1]({ event: name, id: 1, payload });
  };
  try {
    await vi.waitFor(() => expect(renderer.setState).toHaveBeenCalledWith('idle', expect.any(Number)));

    // 1. 收到 DeepSeek 自主漫步事件，播放对应方向动画、特效和想法气泡
    deliverEvent('pet:activity', {
      id: 'deepseek-activity:1',
      phase: 'start',
      state: 'running-right',
      effect: { x: 0.5, y: 0.8, kind: 'sparkle', bounce: 0.85, count: 5 },
      thought: '去桌面右边散散步~',
      thoughtDurationMs: 3600,
    });
    expect(renderer.setState).toHaveBeenLastCalledWith('running-right', expect.any(Number), true);
    expect(renderer.triggerInteractionEffect).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'sparkle' }),
      expect.any(Number),
    );
    expect(stack.hidden).toBe(false);
    expect(stack.textContent).toContain('去桌面右边散散步~');

    // 2. 活动结束后恢复 idle
    deliverEvent('pet:activity', { id: 'deepseek-activity:1', phase: 'end' });
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', expect.any(Number));

    // 3. 自主活动进行中被用户点击，应立即通知后端打断自主移动
    deliverEvent('pet:activity', { id: 'deepseek-activity:2', phase: 'start', state: 'running-left' });
    handle.click({ x: 0.5, y: 0.68, clickCount: 1 });
    expect(interrupt).toHaveBeenCalledTimes(1);

    // 4. 真实待审批提醒到达时抢占自主活动，待审批期间忽略新的自主活动
    const urgent = structuredClone(fixture.expected) as PetSnapshot;
    urgent.revision = 10;
    update(urgent);
    expect(renderer.setState).toHaveBeenLastCalledWith('review', expect.any(Number));
    deliverEvent('pet:activity', {
      id: 'deepseek-activity:3',
      phase: 'start',
      state: 'jumping',
      thought: '不应覆盖审批提醒',
    });
    expect(stack.textContent).toContain('待审批');
    expect(stack.textContent).not.toContain('不应覆盖审批提醒');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('气泡按是否包含操作按钮切换 has-actions 类，并在多行发言增高时同步最新背景与热区', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  let bubbleTop = 140;
  let bubbleHeight = 64;
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      return rect(6, bubbleTop - idx * (bubbleHeight + 6), 228, bubbleHeight);
    }
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = bubbleTop - idx * (bubbleHeight + 6);
    if (this.dataset.action === 'close') return rect(210, top + 6, 18, 18);
    return rect(16, top + bubbleHeight - 26, 52, 20);
  });
  let update!: (value: PetSnapshot) => void;
  const idleSnapshot: PetSnapshot = { revision: 1, sessions: [], notifications: [] };
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
  vi.spyOn(api, 'snapshot').mockResolvedValue(idleSnapshot);
  vi.spyOn(api, 'interruptPetActivity').mockResolvedValue(undefined);
  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn(), triggerInteractionEffect: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  const deliverEvent = (name: string, payload: unknown) => {
    const call = [...vi.mocked(listen).mock.calls].reverse().find(([eventName]) => eventName === name);
    if (!call) throw new Error(`缺少 ${name} 事件监听`);
    call[1]({ event: name, id: 1, payload });
  };
  try {
    handle.pose({ width: 240, height: 440, runs: [[300, 80, 160]] }, { x: 0, y: 0, width: 240, height: 440 });
    await vi.waitFor(() => expect(renderer.setState).toHaveBeenCalledWith('idle', expect.any(Number)));

    // 1. 纯发言气泡（DeepSeek 较长多行想法）不带 has-actions，并按真实增高后的高度同步 backdrops
    bubbleTop = 124;
    bubbleHeight = 110;
    const longThought = '今天桌面好安静呀，我去右边巡逻一圈顺便伸个大大的懒腰，有事随时叫我哦~';
    deliverEvent('pet:activity', {
      id: 'deepseek-activity:long',
      phase: 'start',
      state: 'waving',
      thought: longThought,
      thoughtDurationMs: 5200,
    });
    const thoughtBubble = stack.querySelector<HTMLElement>('aside.bubble')!;
    expect(stack.hidden).toBe(false);
    expect(thoughtBubble.classList.contains('has-actions')).toBe(false);
    expect(thoughtBubble.querySelector('.bubble-detail')?.textContent).toBe(longThought);
    expect(poses.mock.lastCall?.[0].bubbleInput?.backdrops).toEqual([
      {
        x: 6,
        y: 124,
        width: 228,
        height: 110,
        radius: 14,
      },
    ]);

    // 2. 双击互动面板与通知提醒均带有操作按钮，应启用 has-actions；逐个关闭后隐藏堆栈
    bubbleTop = 186;
    bubbleHeight = 54;
    handle.click({ x: 0.5, y: 0.8, clickCount: 2 });
    const panelBubble = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble')).find((el) =>
      el.textContent?.includes('投喂'),
    )!;
    expect(panelBubble.classList.contains('has-actions')).toBe(true);

    const urgent = structuredClone(fixture.expected) as PetSnapshot;
    urgent.revision = 2;
    update(urgent);
    const reminderBubble = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble')).find((el) =>
      el.textContent?.includes('待审批'),
    )!;
    expect(reminderBubble.classList.contains('has-actions')).toBe(true);

    for (const closeBtn of Array.from(stack.querySelectorAll<HTMLButtonElement>('button[data-action="close"]'))) {
      closeBtn.click();
    }
    expect(stack.hidden).toBe(true);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(0);
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('任务完成后支持多气泡向上逐层叠加，新的任务提醒、自主发言与点击互动均可显示并独立操作', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      // 模拟 column-reverse：index 越大越靠上（y 越小），4 层都在窗口内
      return rect(6, 186 - idx * 60, 228, 54);
    }
    if (this.dataset.action === 'close_all') return rect(158, 250, 76, 22);
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = 186 - idx * 60;
    if (this.dataset.action === 'close') return rect(210, top + 6, 18, 18);
    if (this.dataset.action === 'session') return rect(14, top + 30, 60, 20);
    return rect(78, top + 30, 56, 20);
  });

  const keyA = { source: 'codex' as const, installationId: 'i1', sessionId: 'task-a' };
  const keyB = { source: 'deepseek' as const, installationId: 'i2', sessionId: 'task-b' };
  let update!: (value: PetSnapshot) => void;
  const endedA: PetSnapshot = {
    revision: 1,
    sessions: [
      { key: keyA, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 },
    ],
    notifications: [
      {
        id: 'end-a',
        key: keyA,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        summary: '任务 A 已完成',
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
  vi.spyOn(api, 'snapshot').mockResolvedValue(endedA);
  const openSession = vi.spyOn(api, 'openSession').mockResolvedValue({ status: 'requested' });
  vi.spyOn(api, 'interruptPetActivity').mockResolvedValue(undefined);

  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn(), triggerInteractionEffect: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  const deliverEvent = (name: string, payload: unknown) => {
    const call = [...vi.mocked(listen).mock.calls].reverse().find(([eventName]) => eventName === name);
    if (!call) throw new Error(`缺少 ${name} 事件监听`);
    call[1]({ event: name, id: 1, payload });
  };
  try {
    handle.pose({ width: 240, height: 440, runs: [[300, 80, 160]] }, { x: 0, y: 0, width: 240, height: 440 });
    await vi.waitFor(() => expect(stack.querySelectorAll('aside.bubble')).toHaveLength(1));
    expect(stack.classList.contains('is-stacked')).toBe(false);
    expect(stack.querySelectorAll('aside.bubble')[0].textContent).toContain('任务 A 已完成');

    // 任务 A 招手动画播完回到 idle，但任务 A 气泡仍未关闭
    handle.animationEnded('waving', 700);
    expect(renderer.setState).toHaveBeenLastCalledWith('idle', 700);

    // 1. 第二个任务完成：向上叠加第 2 层气泡，两层同时可见
    update({
      revision: 2,
      sessions: [
        endedA.sessions[0],
        { key: keyB, turnId: 't2', status: 'ended', pending: [], unread: true, lastEventAtMs: 200 },
      ],
      notifications: [
        endedA.notifications[0],
        {
          id: 'end-b',
          key: keyB,
          turnId: 't2',
          kind: { type: 'turn_ended', outcome: 'completed' },
          summary: '任务 B 已完成',
          occurredAtMs: 200,
          read: false,
        },
      ],
    });
    handle.animationEnded('waving', 1400);

    let bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(2);
    expect(stack.classList.contains('is-stacked')).toBe(true);
    expect(bubbles[0].textContent).toContain('任务 A 已完成');
    expect(bubbles[1].textContent).toContain('任务 B 已完成');
    expect(poses.mock.lastCall?.[0].bubbleInput?.backdrops).toEqual([
      { x: 6, y: 186, width: 228, height: 54, radius: 14 },
      { x: 6, y: 126, width: 228, height: 54, radius: 14 },
      { x: 158, y: 250, width: 76, height: 22, radius: 11 },
    ]);

    // 2. 任务完成后收到 DeepSeek 新对话：不再被拦截，向上叠加第 3 层气泡
    deliverEvent('pet:activity', {
      id: 'deepseek-activity:after-task',
      phase: 'start',
      state: 'running-right',
      thought: '任务做完啦，我去逛一圈~',
      thoughtDurationMs: 4000,
    });
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(3);
    expect(bubbles[2].textContent).toContain('任务做完啦，我去逛一圈~');

    // 3. 点击桌宠产生新的互动对话：向上叠加第 4 层气泡
    handle.click({ x: 0.5, y: 0.68, clickCount: 1 });
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(4);
    expect(bubbles[3].textContent).toContain('摸摸头');
    expect(poses.mock.lastCall?.[0].bubbleInput?.backdrops).toHaveLength(5);

    // 4. 通过原生点击指定第 2 层（bubble=1）的打开会话按钮，精确命中任务 B
    const stampForOpen = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(stampForOpen, 'session', 1);
    expect(openSession).toHaveBeenCalledWith(keyB, 'session');

    // 5. 关闭第 1 层（bubble=0，任务 A）后，其余 3 个气泡保持可见且自动下移索引
    const stampForClose = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(stampForClose, 'close', 0);
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(3);
    expect(bubbles[0].textContent).toContain('任务 B 已完成');
    expect(bubbles[0].dataset.bubbleIndex).toBe('0');
    expect(bubbles[1].textContent).toContain('任务做完啦，我去逛一圈~');
    expect(bubbles[1].dataset.bubbleIndex).toBe('1');
    expect(bubbles[2].textContent).toContain('摸摸头');
    expect(bubbles[2].dataset.bubbleIndex).toBe('2');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

function createMemoryStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (key: string) => (map.has(key) ? map.get(key)! : null),
    setItem: (key: string, value: string) => {
      map.set(key, String(value));
    },
    removeItem: (key: string) => {
      map.delete(key);
    },
    clear: () => {
      map.clear();
    },
  };
}

it('关闭历史信息气泡后标为已读并持久化关闭状态，重新打开宠物不再重新出现，新提醒仍正常弹出', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  const storage = createMemoryStorage();
  vi.stubGlobal('document', document);
  vi.stubGlobal('localStorage', storage);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.dataset.action === 'close_all') return rect(158, 250, 76, 22);
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      return rect(6, 186 - idx * 66, 228, 60);
    }
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = 186 - idx * 66;
    if (this.dataset.action === 'close') return rect(210, top + 6, 18, 18);
    return rect(14, top + 34, 60, 20);
  });

  const keyHistory = { source: 'codex' as const, installationId: 'i1', sessionId: 'history-session' };
  const keyAttention = { source: 'deepseek' as const, installationId: 'i2', sessionId: 'attention-session' };
  const initialSnapshot: PetSnapshot = {
    revision: 1,
    sessions: [
      { key: keyHistory, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 },
      {
        key: keyAttention,
        turnId: 't2',
        status: 'needs_attention',
        pending: [{ id: 'req-old', reason: 'approval', summary: '旧审批请求' }],
        unread: true,
        lastEventAtMs: 200,
      },
    ],
    notifications: [
      {
        id: 'hist-1',
        key: keyHistory,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        summary: '历史任务完成',
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
  let update!: (value: PetSnapshot) => void;
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockImplementation(async (receive) => {
    update = receive;
    return () => {};
  });
  vi.spyOn(api, 'snapshot').mockResolvedValue(initialSnapshot);
  const markRead = vi.spyOn(api, 'markRead').mockResolvedValue(initialSnapshot);

  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  let handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, vi.fn(), vi.fn());
  try {
    let stack = document.querySelector<HTMLElement>('.bubble-stack')!;
    await vi.waitFor(() => expect(stack.querySelectorAll('aside.bubble')).toHaveLength(2));
    expect(stack.querySelector<HTMLButtonElement>('button[data-action="close_all"]')?.textContent).toBe('全部关闭 (2)');

    // 逐个关闭历史任务气泡与旧待审批气泡
    const firstClose = stack.querySelector<HTMLButtonElement>('aside.bubble[data-bubble-index="0"] button[data-action="close"]')!;
    firstClose.click();
    expect(markRead).toHaveBeenCalledWith(keyHistory);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(1);
    expect(stack.querySelector('button[data-action="close_all"]')).toBeNull();

    const secondClose = stack.querySelector<HTMLButtonElement>('aside.bubble[data-bubble-index="0"] button[data-action="close"]')!;
    secondClose.click();
    expect(markRead).toHaveBeenCalledWith(keyAttention);
    expect(stack.hidden).toBe(true);

    // 模拟退出并重新打开宠物：即使快照仍含未清除的历史或未解除的旧请求，已关闭的气泡也不再重现
    handle.stop();
    renderer.setState.mockClear();
    renderer.setFps.mockClear();
    handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, vi.fn(), vi.fn());
    stack = document.querySelector<HTMLElement>('.bubble-stack')!;
    await vi.waitFor(() => {
      expect(renderer.setFps).toHaveBeenCalledWith(30);
      expect(renderer.setState).toHaveBeenLastCalledWith('review', expect.any(Number));
    });
    expect(stack.hidden).toBe(true);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(0);

    // 同一会话收到新的历史完成通知时仍正常弹出
    update({
      revision: 2,
      sessions: [
        { key: keyHistory, turnId: 't3', status: 'ended', pending: [], unread: true, lastEventAtMs: 300 },
        initialSnapshot.sessions[1],
      ],
      notifications: [
        initialSnapshot.notifications[0],
        {
          id: 'hist-2',
          key: keyHistory,
          turnId: 't3',
          kind: { type: 'turn_ended', outcome: 'completed' },
          summary: '新任务完成',
          occurredAtMs: 300,
          read: false,
        },
      ],
    });
    expect(stack.hidden).toBe(false);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(1);
    expect(stack.textContent).toContain('新任务完成');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('点击全部关闭按钮可一次性关闭所有可见、排队中及互动气泡并批量标为已读', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  const storage = createMemoryStorage();
  vi.stubGlobal('document', document);
  vi.stubGlobal('localStorage', storage);
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.dataset.action === 'close_all') return rect(150, 250, 84, 22);
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      return rect(6, 186 - idx * 60, 228, 54);
    }
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = 186 - idx * 60;
    if (this.dataset.action === 'close') return rect(210, top + 6, 18, 18);
    return rect(14, top + 30, 56, 18);
  });

  const keys = Array.from({ length: 5 }, (_, i) => ({
    source: 'codex' as const,
    installationId: 'i1',
    sessionId: `session-${i + 1}`,
  }));
  const manySnapshot: PetSnapshot = {
    revision: 1,
    sessions: keys.map((key, i) => ({
      key,
      turnId: `t-${i + 1}`,
      status: 'ended' as const,
      pending: [],
      unread: true,
      lastEventAtMs: 100 + i * 10,
    })),
    notifications: keys.map((key, i) => ({
      id: `n-${i + 1}`,
      key,
      turnId: `t-${i + 1}`,
      kind: { type: 'turn_ended' as const, outcome: 'completed' as const },
      summary: `任务 ${i + 1} 已完成`,
      occurredAtMs: 100 + i * 10,
      read: false,
    })),
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
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshot').mockResolvedValue(manySnapshot);
  const markReadMany = vi.spyOn(api, 'markReadMany').mockResolvedValue({
    ...manySnapshot,
    revision: 2,
    sessions: manySnapshot.sessions.map((s) => ({ ...s, unread: false })),
    notifications: manySnapshot.notifications.map((n) => ({ ...n, read: true })),
  });

  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn(), triggerInteractionEffect: vi.fn() };
  let handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  try {
    handle.pose({ width: 240, height: 440, runs: [[300, 80, 160]] }, { x: 0, y: 0, width: 240, height: 440 });
    let stack = document.querySelector<HTMLElement>('.bubble-stack')!;
    await vi.waitFor(() => expect(stack.querySelectorAll('aside.bubble')).toHaveLength(4));

    // 再叠加 1 个点击互动气泡，总计 6 个气泡（屏幕显示 4 个）
    handle.click({ x: 0.5, y: 0.68, clickCount: 1 });
    const closeAllBtn = stack.querySelector<HTMLButtonElement>('button[data-action="close_all"]')!;
    expect(closeAllBtn).toBeTruthy();
    expect(closeAllBtn.textContent).toBe('全部关闭 (6)');
    expect(poses.mock.lastCall?.[0].bubbleInput?.buttons).toContainEqual({
      bubble: 0,
      action: 'close_all',
      rect: { x: 150, y: 250, width: 84, height: 22 },
    });

    // 通过原生气泡点击触发 close_all，一次性清空全部 6 个气泡（含排队中的第 5 条历史提醒）
    const closeAllStamp = poses.mock.lastCall![0].bubbleInput!.stamp;
    deliverBubbleAction(closeAllStamp, 'close_all', 0);

    expect(stack.hidden).toBe(true);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(0);
    expect(stack.querySelector('button[data-action="close_all"]')).toBeNull();
    expect(poses.mock.lastCall?.[0].bubbleInput).toBeUndefined();
    expect(poses.mock.lastCall?.[0].mask.runs).toEqual([[300, 80, 160]]);
    expect(markReadMany).toHaveBeenCalledTimes(1);
    expect(markReadMany.mock.calls[0][0]).toHaveLength(5);

    // 重新打开宠物后，5 条历史提醒全部保持关闭状态
    handle.stop();
    renderer.setState.mockClear();
    renderer.setFps.mockClear();
    handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
    stack = document.querySelector<HTMLElement>('.bubble-stack')!;
    await vi.waitFor(() => {
      expect(renderer.setFps).toHaveBeenCalledWith(30);
      expect(renderer.setState).toHaveBeenCalled();
    });
    expect(stack.hidden).toBe(true);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(0);
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('多个气泡超出容器可用高度时自动按优先级裁剪，避免顶部气泡头部被截断并在关闭或放大窗口后自动补位', async () => {
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  let resizeCallback: (() => void) | undefined;
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(cb: () => void) {
        resizeCallback = cb;
      }
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);

  // 模拟真实 flex-direction: column-reverse 布局：bottom 固定在 75vw + 6px，max-height = 100vh - 75vw - 10px
  let canvasWidth = 240;
  let canvasHeight = 440;
  let bubbleHeight = 92;
  const gap = 4;
  const closeAllHeight = 20;

  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, canvasWidth, canvasHeight);
    const petStageHeight = canvasWidth * 0.75;
    const stackBottom = canvasHeight - petStageHeight - 6;
    const stackMaxHeight = canvasHeight - petStageHeight - 10;
    const bubbleNodes = Array.from(document.querySelectorAll<HTMLElement>('.bubble-stack aside.bubble:not([hidden])'));
    const closeAllNode = document.querySelector<HTMLElement>('.bubble-stack button[data-action="close_all"]:not([hidden])');
    const hasCloseAll = Boolean(closeAllNode);
    const totalContentHeight =
      bubbleNodes.length * bubbleHeight +
      (hasCloseAll ? closeAllHeight : 0) +
      Math.max(0, bubbleNodes.length + (hasCloseAll ? 1 : 0) - 1) * gap;

    if (this.classList.contains('bubble-stack')) {
      const renderedHeight = Math.min(totalContentHeight, stackMaxHeight);
      return rect(6, stackBottom - renderedHeight, canvasWidth - 12, renderedHeight);
    }
    if (this.dataset.action === 'close_all') {
      return rect(canvasWidth - 90, stackBottom - closeAllHeight, 84, closeAllHeight);
    }
    const baseBottom = hasCloseAll ? stackBottom - closeAllHeight - gap : stackBottom;
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      const bottom = baseBottom - idx * (bubbleHeight + gap);
      return rect(6, bottom - bubbleHeight, canvasWidth - 12, bubbleHeight);
    }
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = baseBottom - idx * (bubbleHeight + gap) - bubbleHeight;
    if (this.dataset.action === 'close') return rect(canvasWidth - 28, top + 6, 16, 16);
    return rect(14, top + bubbleHeight - 24, 56, 18);
  });

  const keyOld = { source: 'codex' as const, installationId: 'i1', sessionId: 'old-ended' };
  const keyUrgent = { source: 'deepseek' as const, installationId: 'i2', sessionId: 'urgent-approval' };
  const keyNew = { source: 'antigravity' as const, installationId: 'i3', sessionId: 'new-ended' };
  const threeBubblesSnapshot: PetSnapshot = {
    revision: 1,
    sessions: [
      { key: keyOld, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 },
      {
        key: keyUrgent,
        turnId: 't2',
        status: 'needs_attention',
        pending: [{ id: 'req-1', reason: 'approval', summary: '需要审批的命令' }],
        unread: true,
        lastEventAtMs: 150,
      },
      { key: keyNew, turnId: 't3', status: 'ended', pending: [], unread: true, lastEventAtMs: 200 },
    ],
    notifications: [
      {
        id: 'n-old',
        key: keyOld,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        summary: '旧任务完成',
        occurredAtMs: 100,
        read: false,
      },
      {
        id: 'n-new',
        key: keyNew,
        turnId: 't3',
        kind: { type: 'turn_ended', outcome: 'completed' },
        summary: '最新任务完成',
        occurredAtMs: 200,
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
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshot').mockResolvedValue(threeBubblesSnapshot);
  vi.spyOn(api, 'markRead').mockResolvedValue(threeBubblesSnapshot);

  const canvas = document.querySelector('canvas')!;
  canvas.width = canvasWidth;
  canvas.height = canvasHeight;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  try {
    handle.pose({ width: canvasWidth, height: canvasHeight, runs: [[300, 80, 160]] }, { x: 0, y: 0, width: canvasWidth, height: canvasHeight });
    // 1. 240x440 可用高度 250px，3 个 92px 气泡 + 全部关闭按钮合计 308px，会自动裁剪为 2 个（保留紧急审批 + 最新任务）
    await vi.waitFor(() => expect(stack.querySelectorAll('aside.bubble')).toHaveLength(2));
    let bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles[0].textContent).toContain('需要审批的命令');
    expect(bubbles[0].dataset.bubbleIndex).toBe('0');
    expect(bubbles[1].textContent).toContain('最新任务完成');
    expect(bubbles[1].dataset.bubbleIndex).toBe('1');
    expect(stack.querySelector<HTMLButtonElement>('button[data-action="close_all"]')?.textContent).toBe('全部关闭 (3)');
    expect(stack.classList.contains('has-close-all')).toBe(true);
    // 所有可见气泡的背景与关闭按钮均完整落在容器顶部边界（y >= 4）内，顶部气泡头部无截断
    for (const backdrop of poses.mock.lastCall![0].bubbleInput!.backdrops!) {
      expect(backdrop.y).toBeGreaterThanOrEqual(4);
    }

    // 2. 窗口放大到 150%（360x660，可用高度 380px）后，ResizeObserver 自动展开显示全部 3 个气泡
    canvasWidth = 360;
    canvasHeight = 660;
    canvas.width = canvasWidth;
    canvas.height = canvasHeight;
    resizeCallback?.();
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(3);
    expect(bubbles[0].textContent).toContain('旧任务完成');
    expect(bubbles[1].textContent).toContain('需要审批的命令');
    expect(bubbles[2].textContent).toContain('最新任务完成');

    // 3. 窗口缩小到 75%（180x330，可用高度 185px）时，连 2 个 92px 气泡也会溢出（212px > 185px），自动裁剪为 1 个紧急气泡且仍保留“全部关闭 (3)”
    canvasWidth = 180;
    canvasHeight = 330;
    canvas.width = canvasWidth;
    canvas.height = canvasHeight;
    resizeCallback?.();
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(1);
    expect(bubbles[0].textContent).toContain('需要审批的命令');
    expect(stack.querySelector<HTMLButtonElement>('button[data-action="close_all"]')?.textContent).toBe('全部关闭 (3)');
    expect(stack.classList.contains('has-close-all')).toBe(true);

    // 4. 恢复 100% 且微幅溢出容器顶边（y = 1 < stackTop = 4）时同样自动裁剪，关闭顶部气泡后排队气泡自动补位
    canvasWidth = 240;
    canvasHeight = 440;
    bubbleHeight = 73; // 3 * 73 + 20 + 12 = 251px > 250px -> 顶部气泡 top = 3 < 4
    canvas.width = canvasWidth;
    canvas.height = canvasHeight;
    resizeCallback?.();
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(2);
    expect(bubbles[0].textContent).toContain('需要审批的命令');
    expect(bubbles[1].textContent).toContain('最新任务完成');

    // 关闭“最新任务完成”后，排队中的“旧任务完成”自动补位显示
    bubbles[1].querySelector<HTMLButtonElement>('button[data-action="close"]')!.click();
    bubbles = Array.from(stack.querySelectorAll<HTMLElement>('aside.bubble'));
    expect(bubbles).toHaveLength(2);
    expect(bubbles[0].textContent).toContain('旧任务完成');
    expect(bubbles[1].textContent).toContain('需要审批的命令');
    expect(stack.querySelector<HTMLButtonElement>('button[data-action="close_all"]')?.textContent).toBe('全部关闭 (2)');
  } finally {
    handle.stop();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('任务气泡默认到期自动关闭并释放热区（不标记已读），交互可重置计时，支持关闭或动态调整时长', async () => {
  vi.useFakeTimers();
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  const storage = createMemoryStorage();
  vi.stubGlobal('document', document);
  vi.stubGlobal('localStorage', storage);
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.localName === 'aside') return rect(6, 180, 228, 60);
    if (this.dataset.action === 'close') return rect(210, 186, 18, 18);
    if (this.dataset.action === 'session') return rect(14, 214, 60, 20);
    return rect(78, 214, 56, 20);
  });

  const keyA = { source: 'codex' as const, installationId: 'i1', sessionId: 'auto-close-a' };
  const initial: PetSnapshot = {
    revision: 1,
    sessions: [{ key: keyA, turnId: 't1', status: 'ended', pending: [], unread: true, lastEventAtMs: 100 }],
    notifications: [
      {
        id: 'n-1',
        key: keyA,
        turnId: 't1',
        kind: { type: 'turn_ended', outcome: 'completed' },
        summary: '第一个任务完成',
        occurredAtMs: 100,
        read: false,
      },
    ],
  };
  let receiveSettings!: (value: Settings) => void;
  let receiveSnapshot!: (value: PetSnapshot) => void;
  const basePrefs: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    mutedSources: [],
    petId: 'yueying-cape-dancer',
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(basePrefs);
  vi.spyOn(api, 'settingsEvents').mockImplementation(async (cb) => {
    receiveSettings = cb;
    return () => {};
  });
  vi.spyOn(api, 'snapshots').mockImplementation(async (cb) => {
    receiveSnapshot = cb;
    return () => {};
  });
  vi.spyOn(api, 'snapshot').mockResolvedValue(initial);
  const markRead = vi.spyOn(api, 'markRead').mockResolvedValue(initial);
  const copySession = vi.spyOn(api, 'copySession').mockResolvedValue(undefined);

  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const poses = vi.fn<(pose: import('./pet-bindings').InputPose) => void>();
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, poses, vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  const flushMicrotasks = async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  };
  try {
    handle.pose({ width: 240, height: 440, runs: [[300, 80, 160]] }, { x: 0, y: 0, width: 240, height: 440 });
    await flushMicrotasks();
    expect(stack.hidden).toBe(false);
    expect(stack.textContent).toContain('第一个任务完成');

    // 1. 推进 10 秒后点击“复制 ID”按钮，应重置 15 秒自动关闭计时
    vi.advanceTimersByTime(10_000);
    expect(stack.hidden).toBe(false);
    const copyBtn = stack.querySelector<HTMLButtonElement>('button[data-action="copy"]')!;
    copyBtn.click();
    await flushMicrotasks();
    expect(copySession).toHaveBeenCalledWith(keyA);

    // 再过 10 秒（距最初显示已 20 秒，但距交互仅 10 秒）仍保持显示
    vi.advanceTimersByTime(10_000);
    expect(stack.hidden).toBe(false);

    // 再过 5 秒到达重置后的 15 秒上限，任务气泡自动关闭并释放热区，且不标记已读
    vi.advanceTimersByTime(5_000);
    expect(stack.hidden).toBe(true);
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(0);
    expect(poses.mock.lastCall?.[0].bubbleInput).toBeUndefined();
    expect(poses.mock.lastCall?.[0].mask.runs).toEqual([[300, 80, 160]]);
    expect(markRead).not.toHaveBeenCalled();

    // 无关快照刷新不会重新弹出已自动关闭的气泡
    receiveSnapshot({ ...initial, revision: 2 });
    expect(stack.hidden).toBe(true);

    // 2. 设置 bubbleAutoCloseSeconds = 0 禁用自动关闭，新任务气泡不会自动消失
    receiveSettings({ ...basePrefs, revision: 2, bubbleAutoCloseSeconds: 0 });
    receiveSnapshot({
      revision: 3,
      sessions: [{ key: keyA, turnId: 't2', status: 'ended', pending: [], unread: true, lastEventAtMs: 200 }],
      notifications: [
        ...initial.notifications,
        {
          id: 'n-2',
          key: keyA,
          turnId: 't2',
          kind: { type: 'turn_ended', outcome: 'completed' },
          summary: '第二个任务完成',
          occurredAtMs: 200,
          read: false,
        },
      ],
    });
    expect(stack.hidden).toBe(false);
    expect(stack.textContent).toContain('第二个任务完成');
    vi.advanceTimersByTime(120_000);
    expect(stack.hidden).toBe(false);

    // 3. 动态将自动关闭调整为 5 秒后，当前可见气泡在 5 秒后自动关闭
    receiveSettings({ ...basePrefs, revision: 3, bubbleAutoCloseSeconds: 5 });
    vi.advanceTimersByTime(4_999);
    expect(stack.hidden).toBe(false);
    vi.advanceTimersByTime(1);
    expect(stack.hidden).toBe(true);
    expect(markRead).not.toHaveBeenCalled();
  } finally {
    handle.stop();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});

it('多气泡排队时仅对可见任务气泡计时，前批自动关闭后排队气泡补位并获得完整展示时长', async () => {
  vi.useFakeTimers();
  const { document, window } = parseHTML('<html><body><canvas></canvas></body></html>');
  vi.stubGlobal('document', document);
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height } as DOMRect);
  vi.spyOn(window.HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.localName === 'canvas') return rect(0, 0, 240, 440);
    if (this.dataset.action === 'close_all') return rect(150, 250, 84, 22);
    if (this.localName === 'aside') {
      const idx = Number(this.dataset.bubbleIndex ?? 0);
      return rect(6, 186 - idx * 60, 228, 54);
    }
    const parent = this.closest('aside') as HTMLElement | null;
    const idx = Number(parent?.dataset.bubbleIndex ?? 0);
    const top = 186 - idx * 60;
    if (this.dataset.action === 'close') return rect(210, top + 6, 18, 18);
    return rect(14, top + 30, 56, 18);
  });

  const keys = Array.from({ length: 5 }, (_, i) => ({
    source: 'codex' as const,
    installationId: 'i1',
    sessionId: `queued-session-${i + 1}`,
  }));
  const manySnapshot: PetSnapshot = {
    revision: 1,
    sessions: keys.map((key, i) => ({
      key,
      turnId: `t-${i + 1}`,
      status: 'ended' as const,
      pending: [],
      unread: true,
      lastEventAtMs: 100 + i * 10,
    })),
    notifications: keys.map((key, i) => ({
      id: `qn-${i + 1}`,
      key,
      turnId: `t-${i + 1}`,
      kind: { type: 'turn_ended' as const, outcome: 'completed' as const },
      summary: `排队任务 ${i + 1}`,
      occurredAtMs: 100 + i * 10,
      read: false,
    })),
  };
  const prefs: Settings = {
    revision: 1,
    fps: 30,
    sound: false,
    autostart: false,
    mutedSources: [],
    petId: 'yueying-cape-dancer',
    bubbleAutoCloseSeconds: 5,
    placement: { x: 0, y: 0, scale: 1, monitorId: null },
  };
  vi.spyOn(api, 'settings').mockResolvedValue(prefs);
  vi.spyOn(api, 'settingsEvents').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshots').mockResolvedValue(() => {});
  vi.spyOn(api, 'snapshot').mockResolvedValue(manySnapshot);

  const canvas = document.querySelector('canvas')!;
  canvas.width = 240;
  canvas.height = 440;
  const renderer = { setState: vi.fn(), setFps: vi.fn() };
  const handle = bindReminder(canvas, { renderer } as unknown as ReturnType<typeof mountPet>, vi.fn(), vi.fn());
  const stack = document.querySelector<HTMLElement>('.bubble-stack')!;
  try {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    // 初始同屏展示最新 4 个（任务 2~5），任务 1 在队列中等待
    expect(stack.querySelectorAll('aside.bubble')).toHaveLength(4);
    expect(stack.textContent).not.toContain('排队任务 1');

    // 5 秒后前 4 个自动关闭，排队中的任务 1 自动补位出现
    vi.advanceTimersByTime(5_000);
    const remaining = stack.querySelectorAll('aside.bubble');
    expect(remaining).toHaveLength(1);
    expect(remaining[0].textContent).toContain('排队任务 1');

    // 任务 1 拥有完整的 5 秒展示时长
    vi.advanceTimersByTime(4_999);
    expect(stack.hidden).toBe(false);
    vi.advanceTimersByTime(1);
    expect(stack.hidden).toBe(true);
  } finally {
    handle.stop();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  }
});
