// The bubble: draws the fire and turns mouse input into press / release / drag for the
// gesture machine in Rust. It never decides on its own whether a dictation starts.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Fire, type Visual } from "./fire";
import { chime } from "./sounds";
import type { DictationEvent, BubbleConfig } from "../shared/types";

const root = document.getElementById("bubble")!;
const canvas = document.createElement("canvas");
root.appendChild(canvas);
const ctx = canvas.getContext("2d")!;
const fire = new Fire();

let config: BubbleConfig = { opacity: 1, sounds: true };
let cssSize = 0;
let state: DictationEvent["state"] = "idle";

function resize() {
  cssSize = Math.min(window.innerWidth, window.innerHeight);
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(cssSize * dpr);
  canvas.height = Math.round(cssSize * dpr);
  canvas.style.width = `${cssSize}px`;
  canvas.style.height = `${cssSize}px`;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  frame();
}

let raf = 0;
let last = performance.now();
function frame() {
  cancelAnimationFrame(raf);
  const loop = (now: number) => {
    const dt = Math.min(64, now - last);
    last = now;
    fire.draw(ctx, cssSize, now, dt);
    raf = fire.animating ? requestAnimationFrame(loop) : 0;
  };
  last = performance.now();
  raf = requestAnimationFrame(loop);
}

function visualOf(s: DictationEvent["state"]): Visual {
  switch (s) {
    case "recording":
      return "recording";
    case "transcribing":
    case "inserting":
      return "working";
    case "done":
      return "done";
    case "error":
      return "error";
    default:
      return "idle";
  }
}

function applyConfig(c: BubbleConfig) {
  config = c;
  canvas.style.opacity = String(state === "idle" ? c.opacity : 1);
}

listen<DictationEvent>("dictation-state", (e) => {
  const prev = state;
  state = e.payload.state;
  if (config.sounds) {
    if (state === "recording" && prev !== "recording") chime("start");
    if (prev === "recording" && (state === "transcribing" || state === "idle")) chime("stop");
    if (state === "error") chime("error");
  }
  canvas.title = state === "error" ? e.payload.message ?? "" : "";
  fire.set(visualOf(state));
  if (e.payload.expectedMs) fire.expect(e.payload.expectedMs);
  // The opacity setting is for the bubble at rest; while it works, it is fully there.
  canvas.style.opacity = String(state === "idle" ? config.opacity : 1);
  if (state !== "recording") fire.level = 0;
  frame();
});

listen<{ level: number }>("level", (e) => {
  fire.level = e.payload.level;
});

listen<BubbleConfig>("bubble-config", (e) => applyConfig(e.payload));

// Pointer: press immediately (hold-to-talk starts at once), drag after a few pixels.
const DRAG_SLOP = 5;
let down: { x: number; y: number; dragging: boolean } | null = null;

canvas.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  canvas.setPointerCapture(e.pointerId);
  down = { x: e.screenX, y: e.screenY, dragging: false };
  void invoke("bubble_pointer", { kind: "press" });
});

canvas.addEventListener("pointermove", (e) => {
  if (!down || down.dragging) return;
  if (Math.hypot(e.screenX - down.x, e.screenY - down.y) > DRAG_SLOP) {
    down.dragging = true;
    void invoke("bubble_pointer", { kind: "drag" });
  }
});

const release = (e: PointerEvent) => {
  if (!down) return;
  down = null;
  if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
  void invoke("bubble_pointer", { kind: "release" });
};
canvas.addEventListener("pointerup", release);
canvas.addEventListener("pointercancel", release);

canvas.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  void invoke("open_settings", { page: null });
});

window.addEventListener("resize", resize);
resize();
invoke<BubbleConfig>("bubble_config").then(applyConfig).catch(() => {});
