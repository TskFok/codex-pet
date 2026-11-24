import { invoke } from '@tauri-apps/api/core';
import desktopPackage from '../../package.json';
import type { PlaybackSignal } from '../components/pet';
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
  playback: PlaybackSignal;
  validationState?: string;
  validationFps?: number;
  notificationService:
    | { state: 'starting' | 'online' }
    | { state: 'unavailable'; message: string };
}

export const runtimeInfo = (): Promise<RuntimeInfo> => invoke('runtime_info');

export async function recordReport(
  meter: FrameMeter,
  startedAt: number,
  drawCount: number,
  animation?: { state: string; frameIndex: number; fps: 30 | 60 },
  segmentStartedElapsedMs = 0,
  elapsedMs = performance.now() - startedAt,
): Promise<void> {
  const report = {
    recordedAt: new Date().toISOString(),
    elapsedMs,
    segmentStartedElapsedMs,
    userAgent: navigator.userAgent,
    dependencies: { ...desktopPackage.dependencies, ...desktopPackage.devDependencies },
    documentVisible: !document.hidden,
    drawCount,
    frames: meter.takeReport(),
    animation,
  };
  const runtime = await runtimeInfo();
  const completeReport = {
    ...report,
    runtime,
    runtimeReceivedElapsedMs: performance.now() - startedAt,
  };
  console.info('桌宠运行诊断', completeReport);
  await invoke('record_diagnostic', { report: completeReport });
}
