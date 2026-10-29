import { test } from "node:test";
import assert from "node:assert/strict";

import { keyImage, keyTitle } from "./keys.ts";

test("every state draws a valid SVG", () => {
	for (const phase of ["idle", "recording", "transcribing", "inserting", "done", "error", null] as const) {
		const url = keyImage(phase);
		assert.match(url, /^data:image\/svg\+xml;base64,/);
		const svg = Buffer.from(url.split(",")[1] ?? "", "base64").toString();
		assert.match(svg, /^<svg .*<\/svg>$/);
		assert.doesNotMatch(svg, /NaN|undefined/);
	}
});

test("off, failed and listening are told apart", () => {
	const decode = (p: Parameters<typeof keyImage>[0]) => Buffer.from(keyImage(p).split(",")[1] ?? "", "base64").toString();
	assert.match(decode(null), /#5A5A5A/);
	assert.match(decode("error"), /#F07167/);
	assert.match(decode("recording"), /<rect x=/);
	assert.equal(keyTitle(null), "Amber off");
	assert.equal(keyTitle("recording"), "Listening");
});

test("a key that appears later is drawn at once, and polls never overlap", async () => {
	const { Poller } = await import("./client.ts");
	// No handshake file in this test environment: Amber counts as not reachable.
	process.env.APPDATA = "/nonexistent";
	const poller = new Poller(10_000, 10_000);
	const seen: unknown[] = [];
	const stops: (() => void)[] = [];
	try {
		stops.push(poller.watch((s) => seen.push(["first", s])));
		// Let the first poll finish; concurrent polls meanwhile must not notify twice.
		await Promise.all([poller.poll(), poller.poll()]);
		await new Promise((r) => setTimeout(r, 100));
		assert.deepEqual(seen, [["first", null]]);
		stops.push(poller.watch((s) => seen.push(["second", s])));
		assert.deepEqual(seen, [
			["first", null],
			["second", null],
		]);
	} finally {
		for (const stop of stops) stop();
	}
});
