import streamDeck from "@elgato/streamdeck";

import { Dictate } from "./dictate.js";

streamDeck.actions.registerAction(new Dictate());
streamDeck.connect();
