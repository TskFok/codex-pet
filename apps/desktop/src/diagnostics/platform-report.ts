import { invoke } from '@tauri-apps/api/core';
import desktopPackage from '../../package.json';
import type { FrameMeter } from './frame-meter';

export interface RuntimeInfo {
  os: string;
  arch: string;
  osVersion: string;
  webview: string;
  tauri: string;
  appVersion: string;
  visible: boolean;
  focused: boolean;
  scaleFactor: number;
  phase: string;
  notificationService:
    | { state: 'starting' | 'online' }
    | { state: 'unavailable'; message: string };
}

export const runtimeInfo = (): Promise<RuntimeInfo> => invoke('runtime_info');

export async function recordReport(
  meter: FrameMeter,
  startedAt: number,
  drawCount: number,
): Promise<void> {
  const report = {
    recordedAt: new Date().toISOString(),
    elapsedMs: performance.now() - startedAt,
    runtime: await runtimeInfo(),
    userAgent: navigator.userAgent,
    dependencies: { ...desktopPackage.dependencies, ...desktopPackage.devDependencies },
    documentVisible: !document.hidden,
    drawCount,
    frames: meter.report(),
  };
  console.info('桌宠运行诊断', report);
  await invoke('record_diagnostic', { report });
}
