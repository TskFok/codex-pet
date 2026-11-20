/** 记录实际绘制时刻；P95 使用 nearest-rank，不对慢帧去噪。 */
export class FrameMeter {
  private previous: number | undefined;
  private intervals: number[] = [];

  record(timestampMs: number): void {
    if (!Number.isFinite(timestampMs)) return;
    if (this.previous !== undefined) {
      if (timestampMs <= this.previous) return;
      this.intervals.push(timestampMs - this.previous);
    }
    this.previous = timestampMs;
  }

  breakContinuity(): void {
    this.previous = undefined;
  }

  report(): FrameReport {
    return summarize([...this.intervals]);
  }

  takeReport(): FrameReport {
    const intervals = this.intervals;
    this.intervals = [];
    return summarize(intervals);
  }
}

interface FrameReport {
  intervalsMs: number[];
  p95Ms: number;
}

function summarize(intervalsMs: number[]): FrameReport {
  const sorted = [...intervalsMs].sort((a, b) => a - b);
  return { intervalsMs, p95Ms: sorted[Math.ceil(sorted.length * 0.95) - 1] ?? 0 };
}
