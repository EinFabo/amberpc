import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DictationEvent, InsertMethod } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Segmented, Switch } from "../ui";
import { pretty } from "./Hotkeys";

const METHODS: [InsertMethod, string][] = [
  ["paste", "Paste"],
  ["type", "Type"],
];

export function InsertPage({ s, save, phase }: PageProps) {
  const [last, setLast] = useState<string | null>(null);
  useEffect(() => {
    const un = listen<DictationEvent>("dictation-state", (e) => {
      if (e.payload.state === "error") setLast(e.payload.message ?? null);
      if (e.payload.state === "done") setLast(null);
    });
    return () => void un.then((f) => f());
  }, []);

  return (
    <>
      <h1>Inserting</h1>
      <p className="lead">How the text gets into the window you were in when you started dictating.</p>
      <Section>
        <Row
          label="Method"
          hint={
            s.insert.method === "paste"
              ? "Through the clipboard and Ctrl+V - fast and works almost everywhere."
              : "As keystrokes - leaves the clipboard alone, slower for long text, a few apps drop characters."
          }
        >
          <Segmented value={s.insert.method} options={METHODS} onChange={(v) => save((x) => (x.insert.method = v))} />
        </Row>
        {s.insert.method === "paste" && (
          <Row label="Restore the clipboard" hint="Puts back the text that was on the clipboard before. Images on the clipboard are not kept. Dictations never land in the clipboard history (Win+V).">
            <Switch on={s.insert.restoreClipboard} onChange={(v) => save((x) => (x.insert.restoreClipboard = v))} />
          </Row>
        )}
        <Row label="Space after the text" hint="So the next dictation does not stick to the last word.">
          <Switch on={s.insert.trailingSpace} onChange={(v) => save((x) => (x.insert.trailingSpace = v))} />
        </Row>
      </Section>
      <Section title="Try it">
        <div className="row column">
          <div className="note">
            Click into the field and press {s.hotkey.dictate ? <b>{pretty(s.hotkey.dictate)}</b> : "the dictate button"}, or use the button. Microphone, engine and inserting all
            work if text appears here.
          </div>
          <textarea id="test-field" placeholder="Your dictation lands here…" style={{ minHeight: 110 }} />
          <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
            <button
              className="btn primary"
              // Keeps the focus in the text field, where the text should go.
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                document.getElementById("test-field")?.focus();
                void invoke("toggle_dictation");
              }}
            >
              {phase === "recording" ? "Stop" : "Dictate"}
            </button>
            {last && <span className="note error">{last}</span>}
          </div>
        </div>
      </Section>
    </>
  );
}
