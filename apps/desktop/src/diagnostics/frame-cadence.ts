/** 限制 Canvas 的实际绘制频率；卡顿后跳过已过期的绘制时刻。 */
export class FrameCadence {
  private next: number | undefined;
  private readonly interval: number;

  constructor(fps: 30 | 60 = 30) {
    this.interval = 1000 / fps;
  }

  shouldDraw(timestamp: number): boolean {
    if (this.next !== undefined && timestamp < this.next - 0.5) return false;
    this.next = this.next === undefined || timestamp - this.next >= this.interval
      ? timestamp + this.interval
      : this.next + this.interval;
    return true;
  }
}
