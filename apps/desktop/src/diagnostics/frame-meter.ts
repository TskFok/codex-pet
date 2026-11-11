/** 记录实际绘制时刻；P95 使用 nearest-rank，不对慢帧去噪。 */
export class FrameMeter {
  private previous: number | undefined;
  private readonly intervals: number[] = [];

  record(timestampMs: number): void {
    if (!Number.isFinite(timestampMs)) return;
    if (this.previous !== undefined) {
      if (timestampMs <= this.previous) return;
      this.intervals.push(timestampMs - this.previous);
    }
    this.previous = timestampMs;
  }

  report(): { intervalsMs: number[]; p95Ms: number } {
    const sorted = [...this.intervals].sort((a, b) => a - b);
    return {
      intervalsMs: [...this.intervals],
      p95Ms: sorted[Math.ceil(sorted.length * 0.95) - 1] ?? 0,
    };
  }
}
