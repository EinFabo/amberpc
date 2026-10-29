// Amber's fire, drawn on a canvas - the same shapes and colours as the Android bubble
// (Fire.kt): a flat amber disc with one dark glyph. Flame at rest, level bars while
// listening with sparks as loud as the voice, a fuse around the rim while working, a tick
// when the text is in, red when it failed.

export const AMBER = "#FFB020";
export const EMBER_RED = "#D9480F";
export const SPARK = "#FFD58A";
export const ERROR = "#F07167";
export const INK = "#0A0A0A";

/** Disc radius as a share of half the canvas, leaving room for the fuse. */
const DISC = 0.7;
const FUSE_RADIUS = 1.2;
const FUSE_LAP_MS = 1100;
const FUSE_TAIL = [0.85, 0.55, 0.32, 0.16];
const FUSE_SEGMENT = 0.3;

export type Visual = "idle" | "recording" | "working" | "done" | "error";

/** The flame glyph; [inner] is the tongue cut back out in the disc colour. */
function flame(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, inner: boolean, tip = 1, sway = 0) {
  const scale = inner ? 0.5 : 1;
  const base = cy + r * 0.2 + (inner ? r * 0.06 : 0);
  const rb = r * 0.22 * scale;
  const h = r * 0.62 * scale * tip;
  const tipY = base - h;
  const tipX = cx + sway * r * scale;
  const p = Math.min(1, Math.max(0, (h - rb) / (rb * 0.7)));
  const c1x = rb * (0.552 * (1 - p) + 0.18 * p);
  const c1y = tipY + p * h * 0.28;
  const c2y = base - rb * 0.552 - p * (h - rb) * 0.3;
  ctx.beginPath();
  ctx.moveTo(tipX, tipY);
  ctx.bezierCurveTo(tipX + c1x, c1y, cx + rb, c2y, cx + rb, base);
  ctx.arc(cx, base, rb, 0, Math.PI, false);
  ctx.bezierCurveTo(cx - rb, c2y, tipX - c1x, c1y, tipX, tipY);
  ctx.closePath();
  ctx.fill();
}

function flicker(tMs: number, phase = 0): [number, number] {
  const t = tMs / 1000;
  const tip = 1 + 0.11 * Math.sin(t * 9.1 + phase) + 0.06 * Math.sin(t * 15.7 + 1.3 + phase);
  const sway = 0.07 * Math.sin(t * 5.3 + phase) + 0.035 * Math.sin(t * 12.9 + 0.7);
  return [tip, sway];
}

interface Spark {
  x0: number;
  y0: number;
  rise: number;
  drift: number;
  born: number;
  life: number;
  w: number;
}

export class Fire {
  private sparks: Spark[] = [];
  private pending = 0;
  private bars = [0, 0, 0];
  private visual: Visual = "idle";
  private since = performance.now();
  level = 0;

  set(visual: Visual) {
    if (visual === this.visual) return;
    this.visual = visual;
    this.since = performance.now();
    if (visual === "error") this.sparks = [];
    if (visual === "done") this.burst();
  }

  /** Whether anything moves - at rest nothing does, and no frame is drawn. */
  get animating() {
    return this.visual !== "idle" || this.sparks.length > 0;
  }

  draw(ctx: CanvasRenderingContext2D, size: number, now: number, dt: number) {
    const cx = size / 2;
    const cy = size / 2;
    const r = (size / 2) * DISC;
    ctx.clearRect(0, 0, size, size);

    if (this.visual === "recording") this.listen(now, dt, cx, cy, r, size);
    this.drawSparks(ctx, now, size);

    // Disc
    ctx.fillStyle = this.visual === "error" ? ERROR : AMBER;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = INK;
    const elapsed = now - this.since;
    switch (this.visual) {
      case "idle":
      case "error":
        this.drawFlame(ctx, cx, cy, r, 0);
        break;
      case "recording":
        this.drawBars(ctx, cx, cy, r, dt);
        break;
      case "working":
        this.drawFlame(ctx, cx, cy, r, elapsed);
        this.drawFuse(ctx, cx, cy, r, elapsed);
        break;
      case "done":
        this.drawTick(ctx, cx, cy, r, Math.min(1, elapsed / 220));
        break;
    }
  }

  private drawFlame(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, burningMs: number) {
    const [tip, sway] = burningMs > 0 ? flicker(burningMs) : [1, 0];
    const [innerTip, innerSway] = burningMs > 0 ? flicker(burningMs, 2.1) : [1, 0];
    ctx.fillStyle = INK;
    flame(ctx, cx, cy, r, false, tip, sway);
    ctx.fillStyle = this.visual === "error" ? ERROR : AMBER;
    flame(ctx, cx, cy, r, true, innerTip, innerSway);
  }

  private drawBars(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, dt: number) {
    const weights = [0.7, 1, 0.8];
    const w = r * 0.16;
    const gap = r * 0.12;
    for (let i = 0; i < 3; i++) {
      const target = 0.18 + 0.82 * Math.min(1, this.level * weights[i] * 1.25);
      // Up fast, down slower - reads as a voice, not as noise.
      const k = target > this.bars[i] ? 0.5 : 0.12;
      this.bars[i] += (target - this.bars[i]) * Math.min(1, k * (dt / 16));
      const h = r * 0.9 * this.bars[i];
      const x = cx + (i - 1) * (w + gap) - w / 2;
      roundRect(ctx, x, cy - h / 2, w, h, w / 2);
    }
  }

  private drawFuse(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, elapsed: number) {
    const rr = r * FUSE_RADIUS;
    const head = ((elapsed % FUSE_LAP_MS) / FUSE_LAP_MS) * Math.PI * 2 - Math.PI / 2;
    ctx.lineCap = "round";
    ctx.lineWidth = Math.max(2, r * 0.1);
    FUSE_TAIL.forEach((alpha, i) => {
      ctx.strokeStyle = i === 0 ? SPARK : AMBER;
      ctx.globalAlpha = alpha;
      const end = head - i * FUSE_SEGMENT;
      ctx.beginPath();
      ctx.arc(cx, cy, rr, end - FUSE_SEGMENT, end);
      ctx.stroke();
    });
    ctx.globalAlpha = 1;
  }

  private drawTick(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: number, t: number) {
    const pts: [number, number][] = [
      [cx - r * 0.3, cy + r * 0.02],
      [cx - r * 0.07, cy + r * 0.25],
      [cx + r * 0.33, cy - r * 0.2],
    ];
    const l1 = Math.hypot(pts[1][0] - pts[0][0], pts[1][1] - pts[0][1]);
    const l2 = Math.hypot(pts[2][0] - pts[1][0], pts[2][1] - pts[1][1]);
    let left = (l1 + l2) * easeOut(t);
    ctx.strokeStyle = INK;
    ctx.lineWidth = r * 0.17;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(...pts[0]);
    const a = Math.min(1, left / l1);
    ctx.lineTo(pts[0][0] + (pts[1][0] - pts[0][0]) * a, pts[0][1] + (pts[1][1] - pts[0][1]) * a);
    left -= l1;
    if (left > 0) {
      const b = Math.min(1, left / l2);
      ctx.lineTo(pts[1][0] + (pts[2][0] - pts[1][0]) * b, pts[1][1] + (pts[2][1] - pts[1][1]) * b);
    }
    ctx.stroke();
  }

  private listen(now: number, dt: number, cx: number, cy: number, r: number, size: number) {
    this.pending += Math.pow(Math.min(1, Math.max(0, this.level)), 1.4) * 14 * (dt / 1000);
    while (this.pending >= 1) {
      this.pending -= 1;
      this.emit(now, this.level, cx, cy, r, size);
    }
  }

  private emit(now: number, level: number, cx: number, cy: number, r: number, size: number) {
    if (this.sparks.length >= 24) this.sparks.shift();
    const a = -Math.PI / 2 + (Math.random() - 0.5) * 1.5;
    this.sparks.push({
      x0: cx + Math.cos(a) * r * 0.9,
      y0: cy + Math.sin(a) * r * 0.9,
      // The canvas has little headroom above the disc; sparks stay inside it.
      rise: Math.min(size * 0.3, size * (0.12 + 0.2 * level) * (0.75 + 0.5 * Math.random())),
      drift: Math.cos(a) * size * 0.08,
      born: now,
      life: 480 + Math.random() * 320,
      w: size * (0.028 + 0.012 * Math.random()),
    });
  }

  private burst() {
    // One short burst of sparks around the disc when the text is in.
    const now = performance.now();
    for (let i = 0; i < 8; i++) {
      const a = (i / 8) * Math.PI * 2;
      this.sparks.push({ x0: 0.5 + Math.cos(a) * 0.35, y0: 0.5 + Math.sin(a) * 0.35, rise: -1, drift: a, born: now, life: 320, w: 0 });
    }
  }

  private drawSparks(ctx: CanvasRenderingContext2D, now: number, size: number) {
    this.sparks = this.sparks.filter((s) => now - s.born < s.life);
    ctx.lineCap = "round";
    for (const s of this.sparks) {
      const t = (now - s.born) / s.life;
      if (s.rise < 0) {
        // Burst spark: flies outward from the rim.
        const a = s.drift;
        const d = size * (0.36 + 0.12 * easeOut(t));
        const x = size / 2 + Math.cos(a) * d;
        const y = size / 2 + Math.sin(a) * d;
        ctx.strokeStyle = SPARK;
        ctx.globalAlpha = 1 - t;
        ctx.lineWidth = Math.max(1.5, size * 0.035);
        ctx.beginPath();
        ctx.moveTo(x, y);
        ctx.lineTo(x + Math.cos(a) * size * 0.05, y + Math.sin(a) * size * 0.05);
        ctx.stroke();
        continue;
      }
      const x = s.x0 + s.drift * t;
      const y = s.y0 - s.rise * easeOut(t);
      ctx.strokeStyle = t < 0.5 ? AMBER : EMBER_RED;
      ctx.globalAlpha = 1 - t;
      ctx.lineWidth = s.w;
      ctx.beginPath();
      ctx.moveTo(x, y);
      ctx.lineTo(x, y + size * 0.05 * (1 - t));
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
  ctx.fill();
}

function easeOut(t: number) {
  return 1 - Math.pow(1 - Math.min(1, Math.max(0, t)), 3);
}
