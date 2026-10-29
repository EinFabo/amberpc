// Spoken shortcuts: say the phrase on its own, and the saved text goes in instead.

import { useEffect, useState } from "react";
import type { Snippet } from "../../shared/types";
import type { PageProps } from "../App";
import { Section } from "../ui";

export function SnippetsPage({ s, save }: PageProps) {
  const [trigger, setTrigger] = useState("");
  const [text, setText] = useState("");
  return (
    <>
      <h1>Snippets</h1>
      <p className="lead">
        Say a short phrase on its own - “meine Adresse”, “Signatur” - and its saved text goes in instead, line breaks and all. “Snippet …” or “Baustein …” in front works too. A
        sentence that only contains the phrase stays as you said it.
      </p>
      <Section title="New snippet">
        <div className="row column" style={{ gap: 8 }}>
          <input type="text" value={trigger} placeholder="What you say, e.g. meine Adresse" onChange={(e) => setTrigger(e.target.value)} />
          <textarea value={text} placeholder="What gets inserted" onChange={(e) => setText(e.target.value)} />
          <div>
            <button
              className="btn primary"
              disabled={!trigger.trim() || !text.trim()}
              onClick={() => {
                // The same phrase again replaces the saved one, openly.
                const same = (t: string) => t.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, " ").trim();
                void save((x) => {
                  x.snippets = [{ trigger: trigger.trim(), text }, ...x.snippets.filter((sn) => same(sn.trigger) !== same(trigger))];
                });
                setTrigger("");
                setText("");
              }}
            >
              Add
            </button>
          </div>
        </div>
      </Section>
      {s.snippets.length > 0 && (
        <Section title="Saved">
          {s.snippets.map((sn, i) => (
            <Editable
              key={`${i}:${sn.trigger}`}
              snippet={sn}
              onSave={(next) => save((x) => (x.snippets[i] = next))}
              onRemove={() => save((x) => x.snippets.splice(i, 1))}
            />
          ))}
        </Section>
      )}
    </>
  );
}

function Editable({ snippet, onSave, onRemove }: { snippet: Snippet; onSave: (s: Snippet) => void; onRemove: () => void }) {
  const [text, setText] = useState(snippet.text);
  useEffect(() => setText(snippet.text), [snippet.text]);
  return (
    <div className="row column" style={{ gap: 8 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <div className="label">“{snippet.trigger}”</div>
        <button className="btn danger" onClick={onRemove}>
          Remove
        </button>
      </div>
      <textarea value={text} onChange={(e) => setText(e.target.value)} onBlur={() => text !== snippet.text && text.trim() && onSave({ ...snippet, text })} />
    </div>
  );
}
