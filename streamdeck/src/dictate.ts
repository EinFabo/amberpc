import { action, SingletonAction, type KeyAction, type KeyDownEvent, type KeyUpEvent, type WillAppearEvent, type WillDisappearEvent } from "@elgato/streamdeck";

import { Poller, send, type Status } from "./client.js";
import { keyImage, keyTitle } from "./keys.js";

/** Fast enough that "Listening" shows the moment the key goes down. */
const poller = new Poller(300);

/**
 * Hold to dictate, let go to insert; a short tap starts, the next tap stops. What a press
 * means is decided by Amber's gesture machine - the key only reports down and up.
 */
@action({ UUID: "com.einfabo.amber.dictate" })
export class Dictate extends SingletonAction {
	#stops = new Map<string, () => void>();

	override async onKeyDown(_ev: KeyDownEvent): Promise<void> {
		await send("POST", "/v1/press");
		void poller.poll();
	}

	override async onKeyUp(_ev: KeyUpEvent): Promise<void> {
		await send("POST", "/v1/release");
		void poller.poll();
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
