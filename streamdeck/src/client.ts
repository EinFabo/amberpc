// The way to Amber: port and token from the handshake file Amber writes at every start,
// requests to 127.0.0.1 with the token in its own header.

import { readFile } from "node:fs/promises";
import { join } from "node:path";

export type Phase = "idle" | "recording" | "transcribing" | "inserting" | "done" | "error";

export type Status = { phase: Phase; version: string };

type Connection = { port: number; token: string };

export function handshakePath(): string {
	return join(process.env.APPDATA ?? "", "app.amber.pc", "streamdeck.json");
}

let connection: Connection | null = null;

async function connect(): Promise<Connection | null> {
	if (connection) return connection;
	try {
		const parsed = JSON.parse(await readFile(handshakePath(), "utf8")) as Partial<Connection>;
		if (typeof parsed.port === "number" && typeof parsed.token === "string" && parsed.token !== "") {
			connection = { port: parsed.port, token: parsed.token };
		}
	} catch {
		// Amber has not run yet, or is not installed.
	}
	return connection;
}

/**
 * One request; `null` when Amber cannot be reached. A refused connection or a 401 means
 * Amber restarted with a new port or token - the handshake file is read again, once.
 */
export async function send(method: "GET" | "POST", path: string): Promise<unknown | null> {
	for (let attempt = 0; attempt < 2; attempt++) {
		const c = await connect();
		if (!c) return null;
		try {
			const response = await fetch(`http://127.0.0.1:${c.port}${path}`, {
				method,
				headers: { "x-amber-token": c.token },
				signal: AbortSignal.timeout(1500),
			});
			if (response.status === 401) {
				connection = null;
				continue;
			}
			return response.ok ? await response.json() : null;
		} catch {
			connection = null;
		}
	}
	return null;
}

/**
 * Asks Amber how things stand while at least one key is visible - one poll for all keys,
 * and none at all with no Amber key on the page.
 */
export class Poller {
	#watchers = new Set<(status: Status | null) => void>();
	#timer: NodeJS.Timeout | null = null;
	#last: string | null = null;

	constructor(private readonly intervalMs: number) {}

	watch(watcher: (status: Status | null) => void): () => void {
		this.#watchers.add(watcher);
		if (this.#timer === null) {
			this.#timer = setInterval(() => void this.poll(), this.intervalMs);
			void this.poll();
		}
		return () => {
			this.#watchers.delete(watcher);
			if (this.#watchers.size === 0 && this.#timer !== null) {
				clearInterval(this.#timer);
				this.#timer = null;
				this.#last = null;
			}
		};
	}

	/** Right after a press, so the key does not wait for the next tick. */
	async poll(): Promise<void> {
		const status = (await send("GET", "/v1/status")) as Status | null;
		const key = JSON.stringify(status?.phase ?? null);
		// Redraw only on change: setImage on every tick would flicker on some devices.
		if (key === this.#last) return;
		this.#last = key;
		for (const w of this.#watchers) w(status);
	}
}
