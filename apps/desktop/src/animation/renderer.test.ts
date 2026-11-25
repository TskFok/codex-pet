import { afterEach, expect, it, vi } from 'vitest';
import type { PetManifest } from '../types';
import { PetRenderer } from './renderer';

const manifest: PetManifest = {
  version: 1,
  id: 'yueying',
  name: '月影测试',
  atlas: { path: 'atlas.webp', width: 30, height: 10, cellWidth: 10, cellHeight: 10 },
  animations: {
    idle: {
      loopStart: 0,
      frames: [
        { rect: { x: 0, y: 0, width: 10, height: 10 }, durationMs: 100 },
        { rect: { x: 10, y: 0, width: 10, height: 10 }, durationMs: 170 },
      ],
    },
    waving: { frames: [{ rect: { x: 20, y: 0, width: 10, height: 10 }, durationMs: 200 }] },
  },
};

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((complete) => { resolve = complete; });
  return { promise, resolve };
}

function browser() {
  const callbacks = new Map<number, FrameRequestCallback>();
  const images: ImageStub[] = [];
  const pendingDecode = new Map<string, Promise<void>>();
  let frameId = 0;
  let now = 0;
  class ImageStub {
    src = '';
    constructor() { images.push(this); }
    decode = vi.fn(() => pendingDecode.get(this.src) ?? Promise.resolve());
  }
  vi.stubGlobal('Image', ImageStub);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    callbacks.set(++frameId, callback);
    return frameId;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => callbacks.delete(id));
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  function canvas() {
    const drawImage = vi.fn();
    const context = { clearRect: vi.fn(), drawImage };
    const node = { width: 100, height: 100, getContext: () => context } as unknown as HTMLCanvasElement;
    return { node, drawImage };
  }
  function frame(timestamp: number) {
    now = timestamp;
    const batch = [...callbacks.keys()];
    for (const id of batch) {
      const callback = callbacks.get(id);
      callbacks.delete(id);
      callback?.(timestamp);
    }
  }
  return { callbacks, images, pendingDecode, canvas, frame };
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('默认 30 FPS 只画 0、34ms，卡顿后只画当前姿态并复用已解码图集', async () => {
  const { images, canvas, frame, callbacks } = browser();
  const target = canvas();
  const onDraw = vi.fn();
  const renderer = new PetRenderer({ resolveAtlasUrl: () => '/bundled-atlas.webp', onDraw });
  await renderer.mount(target.node, manifest);
  renderer.setVisible(true, 0);
  [0, 16, 34].forEach(frame);
  expect(onDraw.mock.calls.map(([ms]) => ms)).toEqual([0, 34]);

  frame(5000);
  expect(target.drawImage.mock.calls.map(([, x]) => x)).toEqual([0, 0, 10]);
  expect(callbacks.size).toBe(1);
  renderer.setState('waving', 5001);
  frame(5034);
  expect(target.drawImage).toHaveBeenLastCalledWith(images[0], 20, 0, 10, 10, 0, 0, 100, 100);
  expect(images.map((image) => image.src)).toEqual(['/bundled-atlas.webp']);
  expect(images[0].decode).toHaveBeenCalledTimes(1);
});

it('切换 60 FPS 保留动作时间基准，回到 30 FPS 后仍只有一条循环', async () => {
  const { canvas, frame, callbacks } = browser();
  const target = canvas();
  const renderer = new PetRenderer({ resolveAtlasUrl: (atlas) => atlas.path });
  await renderer.mount(target.node, manifest);
  renderer.setVisible(true, 0);
  frame(90);
  renderer.setFps(60);
  [100, 116.7, 133.4].forEach(frame);
  expect(target.drawImage.mock.calls.map(([, x]) => x)).toEqual([0, 10, 10, 10]);
  renderer.setFps(30);
  [150, 166, 184].forEach(frame);
  expect(target.drawImage).toHaveBeenCalledTimes(6);
  expect(callbacks.size).toBe(1);
});

it('隐藏立即取消待执行回调，恢复时以当前状态从动作开头播放', async () => {
  const { canvas, frame, callbacks } = browser();
  const target = canvas();
  const renderer = new PetRenderer({ resolveAtlasUrl: (atlas) => atlas.path });
  await renderer.mount(target.node, manifest);
  renderer.setVisible(true, 0);
  frame(150);
  renderer.setVisible(false, 160);
  expect(callbacks.size).toBe(0);
  frame(600);
  expect(target.drawImage).toHaveBeenCalledTimes(1);
  renderer.setVisible(true, 600);
  renderer.setVisible(true, 620);
  frame(690);
  frame(708);
  frame(724);
  expect(target.drawImage.mock.calls.map(([, x]) => x)).toEqual([10, 0, 10]);
  renderer.dispose();
  renderer.dispose();
  expect(callbacks.size).toBe(0);
});

it('解码期间的重复挂载与替换只允许最后一次挂载启动，卸载可使待完成的挂载失效', async () => {
  const { canvas, frame, images, pendingDecode, callbacks } = browser();
  const first = canvas();
  const second = canvas();
  const third = canvas();
  const old = deferred();
  const latest = deferred();
  pendingDecode.set('atlas.webp', old.promise);
  pendingDecode.set('new.webp', latest.promise);
  const renderer = new PetRenderer({ resolveAtlasUrl: (atlas) => atlas.path });
  renderer.setVisible(true, 0);
  const older = renderer.mount(first.node, manifest);
  const repeated = renderer.mount(first.node, manifest);
  const next = renderer.mount(second.node, { ...manifest, atlas: { ...manifest.atlas, path: 'new.webp' } });
  latest.resolve();
  await next;
  old.resolve();
  await Promise.all([older, repeated]);
  frame(0);
  expect(first.drawImage).not.toHaveBeenCalled();
  expect(second.drawImage).toHaveBeenCalledTimes(1);
  expect(images.map((image) => image.src)).toEqual(['atlas.webp', 'new.webp']);
  expect(callbacks.size).toBe(1);
  const loading = deferred();
  pendingDecode.set('third.webp', loading.promise);
  const pending = renderer.mount(third.node, { ...manifest, atlas: { ...manifest.atlas, path: 'third.webp' } });
  renderer.dispose();
  loading.resolve();
  await pending;
  frame(36);
  expect(third.drawImage).not.toHaveBeenCalled();
  expect(callbacks.size).toBe(0);
  await renderer.mount(third.node, manifest);
  renderer.setVisible(true, 36);
  frame(36);
  expect(third.drawImage).toHaveBeenCalledTimes(1);
  expect(callbacks.size).toBe(1);
});

it('静止诊断每次显示只画首帧，保留真正绘制回调的时间', async () => {
  const { canvas, callbacks, frame } = browser();
  const target = canvas();
  const onDraw = vi.fn();
  const renderer = new PetRenderer({ resolveAtlasUrl: (atlas) => atlas.path, staticFrame: true, onDraw });
  await renderer.mount(target.node, manifest);
  renderer.setVisible(true, 0);
  frame(0);
  expect(callbacks.size).toBe(0);
  frame(400);
  expect(onDraw.mock.calls.map(([ms]) => ms)).toEqual([0]);
});

it('指定桌宠视口后只绘制并采样底部桌宠区域，上方留白保持透明', async () => {
  const { images, frame } = browser();
  const drawImage = vi.fn();
  const getImageData = vi.fn((_x: number, _y: number, w: number, h: number) => ({
    data: new Uint8ClampedArray(w * h * 4).fill(255),
  }));
  const context = { clearRect: vi.fn(), drawImage, getImageData };
  const node = { width: 240, height: 320, getContext: () => context } as unknown as HTMLCanvasElement;
  const onPose = vi.fn();
  const renderer = new PetRenderer({
    resolveAtlasUrl: (atlas) => atlas.path,
    viewport: () => ({ x: 40, y: 140, width: 160, height: 180 }),
    onPose,
    staticFrame: true,
  });
  await renderer.mount(node, manifest);
  renderer.setVisible(true, 0);
  frame(0);
  expect(drawImage).toHaveBeenLastCalledWith(images[0], 0, 0, 10, 10, 40, 150, 160, 160);
  expect(getImageData).toHaveBeenCalledWith(40, 150, 160, 160);
  expect(onPose).toHaveBeenCalledTimes(1);
  const [mask] = onPose.mock.calls[0];
  expect(mask.width).toBe(240);
  expect(mask.height).toBe(320);
  expect(mask.runs[0]).toEqual([150, 40, 200]);
  expect(mask.runs[mask.runs.length - 1]).toEqual([309, 40, 200]);
});

it('单次动作播放结束后触发结束回调并可在同一帧切回循环动作，避免卡死在末帧', async () => {
  const { images, canvas, frame } = browser();
  const target = canvas();
  const drawn: Array<{ state: string; frameIndex: number }> = [];
  const ended = vi.fn((_state: string, timestamp: number) => {
    renderer.setState('idle', timestamp);
  });
  const renderer = new PetRenderer({
    resolveAtlasUrl: (atlas) => atlas.path,
    onAnimationEnd: ended,
    onDraw: (_ms, state, sample) => drawn.push({ state, frameIndex: sample.frameIndex }),
  });
  await renderer.mount(target.node, manifest);
  renderer.setVisible(true, 0);
  renderer.setState('waving', 0);

  frame(0);
  frame(100);
  expect(ended).not.toHaveBeenCalled();
  expect(target.drawImage).toHaveBeenLastCalledWith(images[0], 20, 0, 10, 10, 0, 0, 100, 100);

  // waving 总时长 200ms，到达 200ms 时切回 idle 并立刻绘制 idle 首帧，随后继续按 idle 推进
  frame(200);
  expect(ended).toHaveBeenCalledTimes(1);
  expect(ended).toHaveBeenCalledWith('waving', 200);
  expect(target.drawImage).toHaveBeenLastCalledWith(images[0], 0, 0, 10, 10, 0, 0, 100, 100);

  frame(310);
  expect(target.drawImage).toHaveBeenLastCalledWith(images[0], 10, 0, 10, 10, 0, 0, 100, 100);
  expect(drawn).toEqual([
    { state: 'waving', frameIndex: 0 },
    { state: 'waving', frameIndex: 0 },
    { state: 'idle', frameIndex: 0 },
    { state: 'idle', frameIndex: 1 },
  ]);

  // 进行中的单次动作显式重播时重新计时，结束后再次触发回调
  renderer.setState('waving', 400);
  frame(450);
  renderer.setState('waving', 500, true);
  frame(650);
  expect(ended).toHaveBeenCalledTimes(1);
  frame(700);
  expect(ended).toHaveBeenCalledTimes(2);
  expect(ended).toHaveBeenLastCalledWith('waving', 700);
});

it('点击弹跳与粒子特效不污染命中掩码缓存，静止诊断模式忽略互动特效', async () => {
  const { frame } = browser();
  const drawImage = vi.fn();
  const getImageData = vi.fn((_x: number, _y: number, w: number, h: number) => ({
    data: new Uint8ClampedArray(w * h * 4).fill(255),
  }));
  const save = vi.fn();
  const restore = vi.fn();
  const context = {
    clearRect: vi.fn(),
    drawImage,
    getImageData,
    save,
    restore,
    translate: vi.fn(),
    rotate: vi.fn(),
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    bezierCurveTo: vi.fn(),
    closePath: vi.fn(),
    fill: vi.fn(),
    stroke: vi.fn(),
  };
  const node = { width: 200, height: 200, getContext: () => context } as unknown as HTMLCanvasElement;
  const onPose = vi.fn();
  const renderer = new PetRenderer({ resolveAtlasUrl: (atlas) => atlas.path, onPose });
  await renderer.mount(node, manifest);
  renderer.setVisible(true, 0);

  // 在首帧未缓存掩码时就触发弹跳特效：仍必须先用原始姿态采样掩码，再用弹跳姿态绘制
  renderer.triggerInteractionEffect({ x: 0.5, y: 0.5, kind: 'heart', bounce: 1 }, 0);
  frame(40);
  expect(onPose).toHaveBeenCalledTimes(1);
  expect(getImageData).toHaveBeenCalledTimes(1);
  // 首帧未命中缓存时会先画原姿态供采样，再重画弹跳姿态
  expect(drawImage).toHaveBeenCalledTimes(2);
  const [canonical, bounced] = drawImage.mock.calls;
  expect(canonical[7]).toBe(200);
  expect(bounced[7]).toBeGreaterThan(200);
  expect(bounced[8]).toBeLessThan(200);
  // 粒子已绘制
  expect(save).toHaveBeenCalled();

  // 下一帧同帧掩码已缓存：不再重复调用 getImageData，也只绘制一次弹跳姿态
  frame(80);
  expect(getImageData).toHaveBeenCalledTimes(1);
  expect(drawImage).toHaveBeenCalledTimes(3);
  expect(renderer.hasAnimation('idle')).toBe(true);
  expect(renderer.hasAnimation('missing')).toBe(false);
});
