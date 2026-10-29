import { useEffect, useState } from "react";
import type { HotkeyMode } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Segmented } from "../ui";

const MODES: [HotkeyMode, string][] = [
  ["both", "Tap or hold"],
  ["toggle", "Tap to toggle"],
  ["hold", "Hold to talk"],
];

const MODE_HINT: Record<HotkeyMode, string> = {
  both: "A short tap starts, the next tap stops. Holding the keys dictates until you let go.",
  toggle: "Press once to start, again to stop.",
  hold: "Dictates while you hold the keys (push to talk).",
};

export function HotkeysPage({ s, save }: PageProps) {
  return (
    <>
      <h1>Hotkeys</h1>
      <p className="lead">Work in every app, also when Amber's windows are closed. Use at least one modifier (Ctrl, Alt, Shift or Win) so normal typing is not caught.</p>
      <Section title="Dictation">
        <Row label="Hotkey">
          <HotkeyInput value={s.hotkey.dictate} onChange={(v) => save((x) => (x.hotkey.dictate = v))} />
        </Row>
        <Row label="Behaviour" hint={MODE_HINT[s.hotkey.mode]}>
          <Segmented value={s.hotkey.mode} options={MODES} onChange={(m) => save((x) => (x.hotkey.mode = m))} />
        </Row>
      </Section>
      <Section title="Bubble">
        <Row label="Show / hide the bubble" hint="Optional.">
          <HotkeyInput value={s.hotkey.toggleBubble} onChange={(v) => save((x) => (x.hotkey.toggleBubble = v))} />
        </Row>
      </Section>
      <p className="note">The bubble works with the mouse as well: click to start and stop, hold to talk, drag to move it, right-click for the settings.</p>
    </>
  );
}

const MODIFIER_CODES = new Set(["ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight", "OSLeft", "OSRight"]);

/** Turns a key press into Tauri's accelerator syntax: "Ctrl+Alt+Space", "Ctrl+Shift+KeyD". */
export function accelerator(e: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  const fkey = /^F([1-9]|1[0-9]|2[0-4])$/.test(e.code);
  // Function keys may stand alone; anything else needs a modifier.
  if (mods.length === 0 && !fkey) return null;
  return [...mods, e.code].join("+");
}

export function pretty(acc: string) {
  return acc
    .split("+")
    .map((k) => k.replace(/^Key([A-Z])$/, "$1").replace(/^Digit(\d)$/, "$1").replace("Super", "Win"))
    .join(" + ");
}

function HotkeyInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [recording, setRecording] = useState(false);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setRecording(false);
        return;
      }
      const acc = accelerator(e);
      if (acc) {
        setRecording(false);
        onChange(acc);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [recording, onChange]);

  return (
    <>
      <button className={`btn hotkey ${recording ? "recording" : ""}`} onClick={() => setRecording(!recording)} onBlur={() => setRecording(false)}>
        {recording ? "Press the keys…" : value ? pretty(value) : "Not set"}
      </button>
      {value && !recording && (
        <button className="btn" onClick={() => onChange("")}>
          Clear
        </button>
      )}
    </>
  );
}
