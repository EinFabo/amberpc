// The key image: Amber's disc in the state of the dictation - the same flame, bars and tick
// as the bubble on screen, drawn as SVG.

import type { Phase } from "./client.js";

const AMBER = "#FFB020";
const SPARK = "#FFD58A";
const ERROR = "#F07167";
const INK = "#0A0A0A";
const OFF = "#5A5A5A";

const SIZE = 144;
const CX = SIZE / 2;
const CY = SIZE / 2;
const R = 50;

/** The flame glyph as an SVG path - the bubble's geometry (fire.ts, Fire.kt). */
function flame(inner: boolean): string {
	const scale = inner ? 0.5 : 1;
	const base = CY + R * 0.2 + (inner ? R * 0.06 : 0);
	const rb = R * 0.22 * scale;
	const h = R * 0.62 * scale;
	const tipY = base - h;
	const p = Math.min(1, Math.max(0, (h - rb) / (rb * 0.7)));
	const c1x = rb * (0.552 * (1 - p) + 0.18 * p);
	const c1y = tipY + p * h * 0.28;
	const c2y = base - rb * 0.552 - p * (h - rb) * 0.3;
	const f = (n: number) => n.toFixed(2);
	return (
		`M${f(CX)},${f(tipY)} C${f(CX + c1x)},${f(c1y)} ${f(CX + rb)},${f(c2y)} ${f(CX + rb)},${f(base)} ` +
		`A${f(rb)},${f(rb)} 0 0 1 ${f(CX - rb)},${f(base)} ` +
		`C${f(CX - rb)},${f(c2y)} ${f(CX - c1x)},${f(c1y)} ${f(CX)},${f(tipY)} Z`
	);
}

function glyph(phase: Phase | null, disc: string): string {
	switch (phase) {
		case "recording": {
			const w = R * 0.16;
			const gap = R * 0.12;
			return [0.55, 0.9, 0.7]
				.map((height, i) => {
					const hh = R * 0.9 * height;
					const x = CX + (i - 1) * (w + gap) - w / 2;
					return `<rect x="${x}" y="${CY - hh / 2}" width="${w}" height="${hh}" rx="${w / 2}" fill="${INK}"/>`;
				})
				.join("");
		}
		case "transcribing":
		case "inserting":
			// The flame, and the fuse arc on the rim.
			return (
				`<path d="${flame(false)}" fill="${INK}"/><path d="${flame(true)}" fill="${disc}"/>` +
				`<circle cx="${CX}" cy="${CY}" r="${R * 1.2}" fill="none" stroke="${SPARK}" stroke-width="7" stroke-linecap="round" stroke-dasharray="70 400"/>`
			);
		case "done":
			return `<polyline points="${CX - R * 0.3},${CY + R * 0.02} ${CX - R * 0.07},${CY + R * 0.25} ${CX + R * 0.33},${CY - R * 0.2}" fill="none" stroke="${INK}" stroke-width="${R * 0.17}" stroke-linecap="round" stroke-linejoin="round"/>`;
		default:
			return `<path d="${flame(false)}" fill="${INK}"/><path d="${flame(true)}" fill="${disc}"/>`;
	}
}

/** `null`: Amber is not running - a grey disc. */
export function keyImage(phase: Phase | null): string {
	const disc = phase === null ? OFF : phase === "error" ? ERROR : AMBER;
	const svg =
		`<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">` +
		`<rect width="${SIZE}" height="${SIZE}" fill="${INK}"/>` +
		`<circle cx="${CX}" cy="${CY}" r="${R}" fill="${disc}"/>` +
		glyph(phase, disc) +
		`</svg>`;
	return `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
}

/** What the title under the image says. */
export function keyTitle(phase: Phase | null): string {
	if (phase === null) return "Amber off";
	return { idle: "", recording: "Listening", transcribing: "…", inserting: "…", done: "", error: "Failed" }[phase];
}
