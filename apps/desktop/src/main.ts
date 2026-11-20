import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { mountPet, PetVisibility, type PlaybackSignal } from './components/pet';
import { FrameMeter } from './diagnostics/frame-meter';
import { recordReport, runtimeInfo } from './diagnostics/platform-report';

const canvas = document.querySelector<HTMLCanvasElement>('#pet')!;
const startedAt = performance.now();
const meter = new FrameMeter();
let drawCount = 0;
let animation: { state: string; frameIndex: number; fps: 30 | 60 } | undefined;
let unlisten: UnlistenFn | undefined;
let dispose: (() => void) | undefined;
let diagnosticTimer: number | undefined;
let segmentStartedElapsedMs = 0;
let stopped = false;

function report(): void {
  const boundary = performance.now() - startedAt;
  void recordReport(meter, startedAt, drawCount, animation, segmentStartedElapsedMs).catch(console.error);
  segmentStartedElapsedMs = boundary;
}

function scheduleReports(): void {
  if (diagnosticTimer !== undefined) window.clearInterval(diagnosticTimer);
  diagnosticTimer = window.setInterval(report, 60_000);
}

async function start(): Promise<void> {
  // 先接收原生事件，再请求快照。按修订号选择最新的事件或快照。
  let pending: PlaybackSignal | undefined;
  let visibility: PetVisibility | undefined;
  const listener = await listen<PlaybackSignal>('pet:playback', (event) => {
    const payload = event.payload;
    if (visibility) visibility.update(payload, performance.now());
    else if (!pending || pending.revision < payload.revision) pending = payload;
  });
  if (stopped) {
    listener();
    return;
  }
  unlisten = listener;
  const runtime = await runtimeInfo();
  if (stopped) return;
  const fps = runtime.validationFps === 60 ? 60 : 30;
  const staticFrame = runtime.phase === 'idle';
  const pet = mountPet(canvas, {
    state: runtime.validationState,
    fps,
    staticFrame,
    onDraw: (timestamp, state, sample) => {
      drawCount += 1;
      animation = { state, frameIndex: sample.frameIndex, fps };
      if (!staticFrame) meter.record(timestamp);
      if (drawCount === 1) {
        report();
        scheduleReports();
      }
    },
  });
  visibility = new PetVisibility(
    (visible, now) => pet.renderer.setVisible(visible, now),
    () => meter.breakContinuity(),
  );

  const documentVisibility = () => visibility?.setDocumentVisible(!document.hidden, performance.now());
  const pointerDown = (event: PointerEvent) => {
    if (runtime.os === 'windows') {
      void invoke('pointer_input', {
        input: { phase: 'down', x: event.offsetX, y: event.offsetY, button: event.button },
      }).catch(console.error);
    }
  };
  document.addEventListener('visibilitychange', documentVisibility);
  canvas.addEventListener('pointerdown', pointerDown);
  dispose = () => {
    document.removeEventListener('visibilitychange', documentVisibility);
    canvas.removeEventListener('pointerdown', pointerDown);
    pet.renderer.dispose();
  };

  documentVisibility();
  visibility.update(pending && pending.revision > runtime.playback.revision ? pending : runtime.playback, performance.now());
  await pet.ready;
  if (stopped) return;
  report();
  if (diagnosticTimer === undefined) scheduleReports();
}

window.addEventListener('pagehide', () => {
  stopped = true;
  if (diagnosticTimer !== undefined) window.clearInterval(diagnosticTimer);
  unlisten?.();
  dispose?.();
}, { once: true });

void start().catch(console.error);
