export type PetEffectKind = 'heart' | 'sparkle' | 'note' | 'star' | 'snack' | 'dizzy';

export interface InteractionEffect {
  x: number;
  y: number;
  kind: PetEffectKind;
  bounce?: number;
  count?: number;
}

export interface BounceTransform {
  scaleX: number;
  scaleY: number;
  offsetY: number;
}

type ParticleShape = 'heart' | 'sparkle' | 'star' | 'note' | 'dizzy';

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  swayAmp: number;
  swayFreq: number;
  swayPhase: number;
  size: number;
  bornAt: number;
  durationMs: number;
  shape: ParticleShape;
  color: string;
  rotation: number;
}

const MAX_PARTICLES = 24;
const BOUNCE_DURATION_MS = 260;

const KIND_SPECS: Record<PetEffectKind, { shapes: ParticleShape[]; colors: string[]; count: number }> = {
  heart: { shapes: ['heart', 'heart', 'sparkle'], colors: ['#ff5c8a', '#ff85a2', '#ffd166'], count: 4 },
  sparkle: { shapes: ['sparkle', 'star', 'sparkle'], colors: ['#ffd166', '#72efdd', '#ffe599'], count: 5 },
  note: { shapes: ['note', 'sparkle', 'note'], colors: ['#5ec8ff', '#9bf6ff', '#ffd166'], count: 4 },
  star: { shapes: ['star', 'sparkle', 'star'], colors: ['#ffb703', '#ffd166', '#fb8500'], count: 5 },
  snack: { shapes: ['heart', 'star', 'sparkle'], colors: ['#ff7b54', '#ffd166', '#ff5c8a'], count: 6 },
  dizzy: { shapes: ['dizzy', 'star', 'dizzy'], colors: ['#c77dff', '#ffd166', '#9d4edd'], count: 5 },
};

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function easeOutCubic(t: number): number {
  const u = 1 - t;
  return 1 - u * u * u;
}

function drawHeart(ctx: CanvasRenderingContext2D, r: number): void {
  ctx.beginPath();
  ctx.moveTo(0, r * 0.85);
  ctx.bezierCurveTo(-r * 1.15, r * 0.15, -r * 1.05, -r * 0.9, 0, -r * 0.25);
  ctx.bezierCurveTo(r * 1.05, -r * 0.9, r * 1.15, r * 0.15, 0, r * 0.85);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
}

function drawSparkle(ctx: CanvasRenderingContext2D, r: number): void {
  const inner = r * 0.28;
  ctx.beginPath();
  for (let i = 0; i < 8; i++) {
    const angle = (i * Math.PI) / 4;
    const radius = i % 2 === 0 ? r : inner;
    const px = Math.cos(angle) * radius;
    const py = Math.sin(angle) * radius;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
}

function drawStar(ctx: CanvasRenderingContext2D, r: number): void {
  const inner = r * 0.45;
  ctx.beginPath();
  for (let i = 0; i < 10; i++) {
    const angle = -Math.PI / 2 + (i * Math.PI) / 5;
    const radius = i % 2 === 0 ? r : inner;
    const px = Math.cos(angle) * radius;
    const py = Math.sin(angle) * radius;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
}

function drawNote(ctx: CanvasRenderingContext2D, r: number): void {
  ctx.beginPath();
  ctx.ellipse(-r * 0.25, r * 0.42, r * 0.36, r * 0.26, -0.35, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(r * 0.08, r * 0.35);
  ctx.lineTo(r * 0.08, -r * 0.72);
  ctx.quadraticCurveTo(r * 0.75, -r * 0.58, r * 0.62, -r * 0.12);
  ctx.stroke();
}

function drawDizzy(ctx: CanvasRenderingContext2D, r: number): void {
  ctx.beginPath();
  for (let i = 0; i <= 18; i++) {
    const t = i / 18;
    const angle = t * Math.PI * 3.2;
    const radius = r * t;
    const px = Math.cos(angle) * radius;
    const py = Math.sin(angle) * radius;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.stroke();
}

export class PetEffects {
  private particles: Particle[] = [];
  private bounce?: { startedAt: number; intensity: number };
  private sequence = 0;

  trigger(effect: InteractionEffect, nowMs: number): void {
    const bounce = effect.bounce ?? 1;
    if (bounce > 0) {
      this.bounce = { startedAt: nowMs, intensity: clamp(bounce, 0.3, 1.6) };
    }
    this.spawnBurst(effect.x, effect.y, effect.kind, nowMs, effect.count);
  }

  spawnBurst(x: number, y: number, kind: PetEffectKind, nowMs: number, count?: number): void {
    const spec = KIND_SPECS[kind];
    const total = clamp(count ?? spec.count, 1, 10);
    const originX = clamp(x, 0.16, 0.84);
    const originY = clamp(y, 0.38, 0.9);
    for (let i = 0; i < total; i++) {
      const seq = this.sequence++;
      const spread = total === 1 ? 0 : (i / (total - 1)) * 2 - 1;
      const jitter = ((seq * 37) % 11 - 5) * 0.008;
      const vx = (kind === 'dizzy' ? spread * 0.14 : spread * 0.11) + jitter;
      const vy = kind === 'dizzy' ? -0.09 - (i % 3) * 0.025 : -0.16 - (i % 3) * 0.035;
      this.particles.push({
        x: originX + spread * 0.04,
        y: originY - (i % 2) * 0.025,
        vx,
        vy,
        swayAmp: kind === 'dizzy' ? 0.045 : 0.02,
        swayFreq: kind === 'dizzy' ? 7.5 : 4.2,
        swayPhase: seq * 1.15,
        size: 0.026 + (i % 3) * 0.005,
        bornAt: nowMs + i * 24,
        durationMs: 820 + (i % 3) * 110,
        shape: spec.shapes[i % spec.shapes.length],
        color: spec.colors[i % spec.colors.length],
        rotation: spread * 0.25,
      });
    }
    if (this.particles.length > MAX_PARTICLES) {
      this.particles.splice(0, this.particles.length - MAX_PARTICLES);
    }
  }

  bounceAt(nowMs: number): BounceTransform | undefined {
    if (!this.bounce) return undefined;
    const elapsed = nowMs - this.bounce.startedAt;
    if (elapsed < 0 || elapsed >= BOUNCE_DURATION_MS) {
      if (elapsed >= BOUNCE_DURATION_MS) this.bounce = undefined;
      return undefined;
    }
    const p = elapsed / BOUNCE_DURATION_MS;
    const k = this.bounce.intensity;
    if (p < 0.35) {
      const t = easeOutCubic(p / 0.35);
      return {
        scaleX: 1 + 0.08 * k * t,
        scaleY: 1 - 0.1 * k * t,
        offsetY: 0,
      };
    }
    if (p < 0.72) {
      const t = (p - 0.35) / 0.37;
      const wave = Math.sin(t * Math.PI);
      const blend = 1 - t;
      return {
        scaleX: 1 + 0.08 * k * blend - 0.05 * k * wave,
        scaleY: 1 - 0.1 * k * blend + 0.07 * k * wave,
        offsetY: -0.045 * k * wave,
      };
    }
    const t = (p - 0.72) / 0.28;
    const settle = (1 - t) * Math.sin(t * Math.PI);
    return {
      scaleX: 1 + 0.02 * k * settle,
      scaleY: 1 - 0.02 * k * settle,
      offsetY: 0,
    };
  }

  isActive(nowMs: number): boolean {
    this.prune(nowMs);
    return this.bounceAt(nowMs) !== undefined || this.particles.length > 0;
  }

  drawParticles(ctx: CanvasRenderingContext2D, width: number, height: number, nowMs: number): void {
    this.prune(nowMs);
    if (this.particles.length === 0 || typeof ctx.save !== 'function') return;
    const unit = Math.min(width, height);
    ctx.save();
    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';
    for (const p of this.particles) {
      const age = nowMs - p.bornAt;
      if (age < 0) continue;
      const t = clamp(age / p.durationMs, 0, 1);
      const seconds = age / 1000;
      const px = (p.x + p.vx * seconds + Math.sin(seconds * p.swayFreq + p.swayPhase) * p.swayAmp) * width;
      const py = (p.y + p.vy * seconds) * height;
      const pop = t < 0.18 ? easeOutCubic(t / 0.18) : 1;
      const fade = t > 0.62 ? 1 - (t - 0.62) / 0.38 : 1;
      const radius = Math.max(3, p.size * unit * pop);
      ctx.save();
      ctx.globalAlpha = clamp(fade, 0, 1);
      ctx.translate(px, py);
      ctx.rotate(p.rotation + Math.sin(seconds * 5 + p.swayPhase) * 0.15);
      ctx.fillStyle = p.color;
      ctx.strokeStyle = p.shape === 'dizzy' ? p.color : 'rgba(255, 255, 255, 0.92)';
      ctx.lineWidth = Math.max(1.2, radius * 0.22);
      if (p.shape === 'heart') drawHeart(ctx, radius);
      else if (p.shape === 'sparkle') drawSparkle(ctx, radius);
      else if (p.shape === 'star') drawStar(ctx, radius);
      else if (p.shape === 'note') {
        ctx.strokeStyle = p.color;
        drawNote(ctx, radius);
      } else {
        drawDizzy(ctx, radius);
      }
      ctx.restore();
    }
    ctx.restore();
  }

  clear(): void {
    this.particles = [];
    this.bounce = undefined;
  }

  private prune(nowMs: number): void {
    if (this.particles.length === 0) return;
    this.particles = this.particles.filter((p) => nowMs - p.bornAt < p.durationMs);
  }
}
