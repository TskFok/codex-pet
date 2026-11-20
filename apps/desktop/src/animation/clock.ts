import type { Animation, FrameRect } from '../types';

export interface AnimationSample {
  frameIndex: number;
  rect: FrameRect;
}

export class AnimationClock {
  private readonly timelines = new WeakMap<Animation, number[]>();

  sample(animation: Animation, elapsedMs: number): AnimationSample {
    let ends = this.timelines.get(animation);
    if (!ends) {
      let end = 0;
      ends = animation.frames.map((frame) => end += frame.durationMs);
      this.timelines.set(animation, ends);
    }

    const last = ends.length - 1;
    const duration = ends[last];
    let time = Math.max(0, elapsedMs);
    if (time >= duration) {
      if (animation.loopStart === undefined) {
        return { frameIndex: last, rect: animation.frames[last].rect };
      }
      const loopTime = ends[animation.loopStart - 1] ?? 0;
      time = loopTime + (time - loopTime) % (duration - loopTime);
    }

    // 找第一个尚未结束的帧，边界时刻归下一帧。
    let first = 0;
    let end = last;
    while (first < end) {
      const middle = Math.floor((first + end) / 2);
      if (ends[middle] <= time) first = middle + 1;
      else end = middle;
    }
    return { frameIndex: first, rect: animation.frames[first].rect };
  }
}
