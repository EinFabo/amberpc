// The last dictations, kept on this PC: nothing dictated into the wrong window, or that
// could not be inserted, is lost.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DictationEvent, HistoryEntry } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Switch } from "../ui";

export function HistoryPage({ s, save, toast }: PageProps) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const load = async () => setEntries(await invoke<HistoryEntry[]>("history_list"));

  useEffect(() => {
    void load();
    // A finished dictation shows up at once.
    const un = listen<DictationEvent>("dictation-state", (e) => {
      if (e.payload.state === "done" || e.payload.state === "error") void load();
    });
    return () => void un.then((f) => f());
  }, []);

  const again = async () => {
    setBusy(true);
    try {
      await invoke("recognize_again");
      toast("Recognized again - the text is on the clipboard.", true);
      void load();
    } catch (e) {
      toast(String(e));
    }
    setBusy(false);
  };

  return (
    <>
      <h1>History</h1>
      <p className="lead">Your last dictations, kept on this PC only and encrypted for your Windows account. Copy one to put it where it should have gone.</p>
      <Section>
        <Row label="Keep a history" hint="Off deletes what is kept.">
          <Switch on={s.history} onChange={(v) => save((x) => (x.history = v))} />
        </Row>
        {s.history && (
          <Row label="Keep dictations for">
            <select value={s.historyDays} onChange={(e) => save((x) => (x.historyDays = Number(e.target.value)))}>
              {[1, 3, 7, 14, 30, 90].map((d) => (
                <option key={d} value={d}>
                  {d === 1 ? "1 day" : `${d} days`}
                </option>
              ))}
            </select>
          </Row>
        )}
        <Row label="Recognize again" hint="When a recognition failed (no connection, a refused key), the recording is kept for ten minutes - in memory only.">
          <button className="btn" disabled={busy} onClick={again}>
            {busy ? "Recognizing…" : "Recognize again"}
          </button>
        </Row>
      </Section>
      {s.history && (
        <Section title={entries.length ? `${entries.length} dictations` : "Nothing yet"}>
          {entries.map((e) => (
            <div className="item" key={e.at} style={{ alignItems: "flex-start" }}>
              <div className="name">
                <div className="hint">
                  {new Date(e.at).toLocaleString()} · {e.program || "unknown program"}
                  {!e.inserted && " · not inserted"}
                </div>
                <div style={{ whiteSpace: "pre-wrap", userSelect: "text", marginTop: 2 }}>{e.text}</div>
              </div>
              <button
                className="btn"
                onClick={async () => {
                  await invoke("copy_text", { text: e.text });
                  toast("Copied.", true);
                }}
              >
                Copy
              </button>
              <button
                className="btn danger"
                onClick={async () => {
                  await invoke("history_delete", { at: e.at });
                  void load();
                }}
              >
                Delete
              </button>
            </div>
          ))}
          {entries.length > 0 && (
            <div className="row">
              <button
                className="btn danger"
                onClick={async () => {
                  await invoke("history_clear");
                  void load();
                }}
              >
                Delete all
              </button>
            </div>
          )}
        </Section>
      )}
    </>
  );
}
