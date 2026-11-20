import { expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { FrameMeter } from './frame-meter';
import { recordReport, type RuntimeInfo } from './platform-report';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

it('异步运行信息返回前仍可绘制，边界慢帧只进入下一段一次', async () => {
  const runtime: RuntimeInfo = {
    os: 'macos', arch: 'aarch64', osVersion: '27.0.1',
    webview: '27.0.1', tauri: '2.12.4', appVersion: '0.1.0',
    visible: true, focused: false, scaleFactor: 2, phase: 'animation',
    playback: { revision: 0, visible: true, suspended: false },
    notificationService: { state: 'online' },
  };
  let finishFirst!: (value: RuntimeInfo) => void;
  const delayed = new Promise<RuntimeInfo>((resolve) => { finishFirst = resolve; });
  vi.mocked(invoke).mockImplementationOnce(() => delayed).mockResolvedValueOnce(undefined);
  const document = { hidden: false };
  vi.stubGlobal('document', document);
  vi.stubGlobal('navigator', { userAgent: 'test-webview' });
  const consoleInfo = vi.spyOn(console, 'info').mockImplementation(() => {});
  try {
    const meter = new FrameMeter();
    [0, 34, 67].forEach((ms) => meter.record(ms));
    const first = recordReport(meter, 0, 3);
    [201, 234].forEach((ms) => meter.record(ms));
    document.hidden = true;
    finishFirst(runtime);
    await first;
    expect(invoke).toHaveBeenNthCalledWith(2, 'record_diagnostic', {
      report: expect.objectContaining({
        runtime, drawCount: 3, documentVisible: true,
        frames: { intervalsMs: [34, 33], p95Ms: 34 },
      }),
    });

    vi.mocked(invoke).mockResolvedValueOnce(runtime).mockResolvedValueOnce(undefined);
    await recordReport(meter, 0, 5);
    expect(invoke).toHaveBeenNthCalledWith(4, 'record_diagnostic', {
      report: expect.objectContaining({
        runtime, drawCount: 5, documentVisible: false,
        frames: { intervalsMs: [134, 33], p95Ms: 134 },
      }),
    });
  } finally {
    consoleInfo.mockRestore();
    vi.unstubAllGlobals();
  }
});
