export interface FrameRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface AnimationFrame {
  rect: FrameRect;
  durationMs: number;
}

export interface Animation {
  frames: AnimationFrame[];
  loopStart?: number;
}

export interface Atlas {
  path: string;
  width: number;
  height: number;
  cellWidth: number;
  cellHeight: number;
}

export interface PetManifest {
  version: 1;
  id: string;
  name: string;
  atlas: Atlas;
  animations: Record<string, Animation>;
  lookDirections?: FrameRect[];
}
