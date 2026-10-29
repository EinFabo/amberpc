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
