import { invoke } from '@tauri-apps/api/core';
import { FrameCadence } from './diagnostics/frame-cadence';
import { FrameMeter } from './diagnostics/frame-meter';
import { recordReport, runtimeInfo } from './diagnostics/platform-report';

const canvas = document.querySelector<HTMLCanvasElement>('#pet')!;
const context = canvas.getContext('2d')!;
const startedAt = performance.now();
let meter = new FrameMeter();
let drawCount = 0;
let visible = false;
let phase = 'animation';
let operatingSystem = '';
let frame = 0;
let cadence = new FrameCadence();

function draw(timestamp: number): void {
  if (!visible || document.hidden) return;
  context.clearRect(0, 0, 160, 180);
  const movement = phase === 'idle' ? 0 : Math.sin(timestamp / 280);
  // 几何测试角色；实体外形与 Rust 命中椭圆一致，轨迹仅作视觉参考。
  context.fillStyle = '#a9d6c4';
  context.beginPath();
  context.ellipse(80, 84, 52, 60, 0, 0, Math.PI * 2);
  context.fill();
  context.fillStyle = '#173f38';
  context.beginPath();
  context.arc(62 + movement * 3, 72, 4, 0, Math.PI * 2);
  context.arc(98 + movement * 3, 72, 4, 0, Math.PI * 2);
  context.fill();
  context.lineWidth = 3;
  context.strokeStyle = '#173f38';
  context.beginPath();
  context.arc(80, 85, 15, 0.15, Math.PI - 0.15);
  context.stroke();
  context.strokeStyle = '#a9d6c499';
  context.beginPath();
  context.moveTo(50, 121);
  context.lineTo(80 + movement * 20, 121);
  context.stroke();
  drawCount += 1;
  if (phase !== 'idle') meter.record(timestamp);
}

function tick(timestamp: number): void {
  if (!visible || document.hidden) return;
  if (cadence.shouldDraw(timestamp)) draw(timestamp);
  if (phase !== 'idle') frame = requestAnimationFrame(tick);
}

function updateVisibility(next: boolean): void {
  if (next === visible) return;
  visible = next;
  cancelAnimationFrame(frame);
  cadence = new FrameCadence();
  // 隐藏间隔不能被误算为绘制卡顿；恢复后开始新测量段。
  if (visible) {
    meter = new FrameMeter();
    frame = requestAnimationFrame(tick);
  }
}

async function refresh(): Promise<void> {
  const runtime = await runtimeInfo();
  phase = runtime.phase;
  operatingSystem = runtime.os;
  updateVisibility(runtime.visible && !document.hidden);
}

canvas.addEventListener('pointerdown', (event) => {
  if (operatingSystem === 'windows') {
    void invoke('pointer_input', { input: { phase: 'down', x: event.offsetX, y: event.offsetY, button: event.button } }).catch(console.error);
  }
});

document.addEventListener('visibilitychange', () => {
  if (document.hidden) updateVisibility(false);
  else void refresh().catch(console.error);
});

async function start(): Promise<void> {
  await refresh();
  await recordReport(meter, startedAt, drawCount);
}
void start().catch(console.error);
setInterval(() => void refresh().catch(console.error), 1000);
setInterval(() => {
  void recordReport(meter, startedAt, drawCount)
    .then(() => { meter = new FrameMeter(); })
    .catch(console.error);
}, 60_000);
