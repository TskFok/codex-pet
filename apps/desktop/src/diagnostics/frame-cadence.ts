/** 仅为窗口实验限制 30 FPS；恢复后丢弃过期帧，避免补画突发。 */
export class FrameCadence {
  private next: number | undefined;
  private readonly interval = 1000 / 30;

  shouldDraw(timestamp: number): boolean {
    if (this.next !== undefined && timestamp < this.next - 0.5) return false;
    this.next = this.next === undefined || timestamp - this.next >= this.interval
      ? timestamp + this.interval
      : this.next + this.interval;
    return true;
  }
}
