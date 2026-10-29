import { action, SingletonAction, type KeyAction, type KeyDownEvent, type KeyUpEvent, type WillAppearEvent, type WillDisappearEvent } from "@elgato/streamdeck";

import { Poller, send, type Status } from "./client.js";
import { keyImage, keyTitle } from "./keys.js";

/** Fast enough that "Listening" shows the moment the key goes down; calm while Amber is off. */
const poller = new Poller(300, 2000);

/**
 * Hold to dictate, let go to insert; a short tap starts, the next tap stops. What a press
 * means is decided by Amber's gesture machine - the key only reports down and up.
 */
@action({ UUID: "com.einfabo.amber.dictate" })
export class Dictate extends SingletonAction {
	#stops = new Map<string, () => void>();
	/**
	 * The SDK does not wait for a handler before calling the next one, so a quick tap could
	 * send "release" before "press" has arrived. Requests go out strictly one after another.
	 */
	#queue: Promise<unknown> = Promise.resolve();

	#send(path: string): Promise<void> {
		this.#queue = this.#queue.then(() => send("POST", path)).catch(() => null);
		return this.#queue.then(() => void poller.poll());
	}

	override async onKeyDown(_ev: KeyDownEvent): Promise<void> {
		await this.#send("/v1/press");
	}

	override async onKeyUp(_ev: KeyUpEvent): Promise<void> {
		await this.#send("/v1/release");
	}

	override onWillAppear(ev: WillAppearEvent): void {
		const key = ev.action as KeyAction;
		this.#stops.get(key.id)?.();
		this.#stops.set(key.id, poller.watch((status) => void draw(key, status)));
	}

	override onWillDisappear(ev: WillDisappearEvent): void {
		this.#stops.get(ev.action.id)?.();
		this.#stops.delete(ev.action.id);
	}
}

async function draw(key: KeyAction, status: Status | null): Promise<void> {
	const phase = status?.phase ?? null;
	await key.setImage(keyImage(phase));
	await key.setTitle(keyTitle(phase));
}
