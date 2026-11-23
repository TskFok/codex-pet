import { parseHTML } from 'linkedom';
import { expect, it, vi } from 'vitest';
import { api } from '../api';
import { invoke } from '@tauri-apps/api/core';
import { bindReminder } from './pet-bindings';
import type { mountPet } from './pet';
import type { PetSnapshot, Settings } from '../types';
import fixture from '../../../../tests/fixtures/events/child-folding.json';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

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
