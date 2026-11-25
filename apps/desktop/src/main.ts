import './styles.css';
import { mountNotifications } from './components/notifications';
import { mountSettings } from './components/settings';
import { bindReminder, type InputPose } from './components/pet-bindings';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { bindPetInput, mountPet, PetVisibility, type PlaybackSignal } from './components/pet';
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
  void recordReport(meter, startedAt, drawCount, animation, segmentStartedElapsedMs, boundary).catch(console.error);
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
  const runtime = await runtimeInfo().catch(async () => ({
    os: /windows/i.test(navigator.userAgent) ? 'windows' : 'macos', phase: 'animation',
    playback: await invoke<PlaybackSignal>('get_playback'), validationState: undefined, validationFps: undefined,
  }));
  if (stopped) return;
  document.body.classList.toggle('native-blur', runtime.os === 'macos');
  let fps: 30 | 60 = runtime.validationFps === 60 ? 60 : 30;
  const staticFrame = runtime.phase === 'idle';
  // IPC 顺序固定。原生线程忙时合并为最新姿态；单次失败后也能恢复静止帧。
  let pendingMask: InputPose | undefined;
  let updating = false;
  let retry: number | undefined;
  async function updateRegion(): Promise<void> {
    if (updating || stopped) return;
    updating = true;
    while (pendingMask && !stopped) {
      const mask = pendingMask;
      pendingMask = undefined;
      try {
        await invoke('update_hit_region', mask);
      } catch (error) {
        console.error(error);
        pendingMask ??= mask;
        retry = window.setTimeout(() => { retry = undefined; void updateRegion(); }, 250);
        break;
      }
    }
    updating = false;
  }
  let reminder: ReturnType<typeof bindReminder> | undefined;
  const pet = mountPet(canvas, {
    state: runtime.validationState,
    fps,
    staticFrame,
    onPose: (mask, frame) => {
      reminder?.pose(mask, frame);
    },
    onAnimationEnd: (state, timestamp) => {
      reminder?.animationEnded(state, timestamp);
    },
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
  reminder = bindReminder(canvas,pet,pose => {pendingMask=pose; if (retry===undefined) void updateRegion();},
    value => {fps=value;},runtime.validationState,runtime.validationFps);
  visibility = new PetVisibility(
    (visible, now) => pet.renderer.setVisible(visible, now),
    () => meter.breakContinuity(),
  );

  const documentVisibility = () => visibility?.setDocumentVisible(!document.hidden, performance.now());
  const unbindInput = bindPetInput(
    window,
    canvas,
    runtime.os,
    (command) => invoke(command),
    (event) => reminder?.click(event),
  );
  const resize = () => pet.renderer.resize(Math.max(1, Math.round(canvas.clientWidth * devicePixelRatio)),
    Math.max(1, Math.round(canvas.clientHeight * devicePixelRatio)));
  const observer = new ResizeObserver(resize);
  observer.observe(canvas);
  let density = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
  function onDensityChange() {
    resize();
    density = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
    density.addEventListener('change', onDensityChange, { once: true });
  }
  density.addEventListener('change', onDensityChange, { once: true });
  document.addEventListener('visibilitychange', documentVisibility);
  dispose = () => {
    document.removeEventListener('visibilitychange', documentVisibility);
    unbindInput();
    observer.disconnect();
    reminder?.stop();
    density.removeEventListener('change', onDensityChange);
    window.clearTimeout(retry);
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

const panelTitle = { '#notifications': '通知', '#settings': '设置' }[window.location.hash];
if (panelTitle) {
  document.title = panelTitle;
  const isNotifications = window.location.hash === '#notifications';
  document.body.className = isNotifications ? 'panel panel-notifications' : 'panel panel-settings';
  const root = document.createElement('main'); canvas.replaceWith(root);
  const mount = isNotifications ? mountNotifications : mountSettings;
  void mount(root).then(stop => { if (stopped) stop(); else dispose = stop; }).catch(e => { root.textContent = String(e); });
} else {
  void start().catch(console.error);
}
