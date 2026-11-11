import { describe, expect, it } from 'vitest';
import { FrameMeter } from './frame-meter';

describe('frame-meter', () => {
  it('记录真实绘制间隔，33、34、33 毫秒满足 30 FPS 预算', () => {
    const meter = new FrameMeter();
    [0, 33, 67, 100].forEach((timestamp) => meter.record(timestamp));
    expect(meter.report().intervalsMs).toEqual([33, 34, 33]);
    expect(meter.report().p95Ms).toBe(34);
    expect(meter.report().p95Ms).toBeLessThanOrEqual(50);
  });

  it('P95 保留慢帧，不能使用平均值掩盖卡顿', () => {
    const meter = new FrameMeter();
    [0, 33, 66, 99, 299].forEach((timestamp) => meter.record(timestamp));
    expect(meter.report().p95Ms).toBe(200);
  });

  it('空样本无间隔；非法、重复与倒退时间不污染基线', () => {
    const meter = new FrameMeter();
    expect(meter.report()).toEqual({ intervalsMs: [], p95Ms: 0 });
    [10, NaN, Infinity, 9, 10, 43].forEach((timestamp) => meter.record(timestamp));
    expect(meter.report()).toEqual({ intervalsMs: [33], p95Ms: 33 });
  });

  it('报告是快照，外部修改不能破坏测量', () => {
    const meter = new FrameMeter();
    [0, 33].forEach((timestamp) => meter.record(timestamp));
    meter.report().intervalsMs.push(999);
    expect(meter.report().intervalsMs).toEqual([33]);
  });
});
