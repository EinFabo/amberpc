// A profile per program: the writing style the smart cleanup uses there, an own instruction,
// and whether the smart cleanup runs there at all.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CleanupChoice, Profile, Program, WritingStyle } from "../../shared/types";
import type { PageProps } from "../App";
import { Section, Segmented } from "../ui";

export const STYLES: [WritingStyle, string][] = [
  ["asSpoken", "As spoken"],
  ["casual", "Casual"],
  ["formal", "Formal"],
  ["concise", "Concise"],
];

const CHOICES: [CleanupChoice, string][] = [
  ["default", "As under Cleanup"],
  ["always", "Always"],
  ["never", "Never"],
];

export function ProgramsPage({ s, save, go }: PageProps) {
  const [running, setRunning] = useState<Program[]>([]);
  const [pick, setPick] = useState("");
  useEffect(() => {
    void invoke<Program[]>("running_programs").then(setRunning);
  }, []);

  const update = (exe: string, change: (p: Profile) => void) =>
    save((x) => {
      const p = x.profiles.find((q) => q.exe === exe);
      if (p) change(p);
    });

  const addable = running.filter((r) => !s.profiles.some((p) => p.exe === r.exe));

  return (
    <>
      <h1>Programs</h1>
      <p className="lead">
        The same words read differently in a chat, a mail and a note. Here each program gets its style - used whenever the smart cleanup runs (
        <a className="link" onClick={() => go("cleanup")}>
          Cleanup
        </a>
        {s.cleanup.smart ? ", now on" : ", now off - “Always” below switches it on for one program"}). Programs without a profile get the text as spoken.
      </p>
      <Section title="Add a program">
        <div className="row">
          <select value={pick} onChange={(e) => setPick(e.target.value)} style={{ flex: 1 }}>
            <option value="">Pick an open program…</option>
            {addable.map((r) => (
              <option key={r.exe} value={r.exe}>
                {r.exe} - {r.title.slice(0, 50)}
              </option>
            ))}
          </select>
          <button
            className="btn primary"
            disabled={!pick}
            onClick={() => {
              const label = pick.replace(/\.exe$/i, "");
              void save((x) => x.profiles.unshift({ exe: pick, label: label.charAt(0).toUpperCase() + label.slice(1), style: "asSpoken", instruction: "", cleanup: "default" }));
              setPick("");
            }}
          >
            Add
          </button>
        </div>
      </Section>
      {s.profiles.length > 0 && (
        <Section>
          {s.profiles.map((p) => (
            <div className="row column" key={p.exe} style={{ gap: 10 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <div className="label">
                  <div>{p.label}</div>
                  <div className="hint">{p.exe}</div>
                </div>
                <Segmented value={p.style} options={STYLES} onChange={(v) => update(p.exe, (q) => (q.style = v))} />
                <button className="btn danger" onClick={() => save((x) => (x.profiles = x.profiles.filter((q) => q.exe !== p.exe)))}>
                  Remove
                </button>
              </div>
              <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                <span className="note">Smart cleanup</span>
                <Segmented value={p.cleanup} options={CHOICES} onChange={(v) => update(p.exe, (q) => (q.cleanup = v))} />
                <Instruction value={p.instruction} onSave={(v) => update(p.exe, (q) => (q.instruction = v))} />
              </div>
            </div>
          ))}
        </Section>
      )}
    </>
  );
}

function Instruction({ value, onSave }: { value: string; onSave: (v: string) => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <input
      type="text"
      style={{ flex: 1 }}
      maxLength={2000}
      value={text}
      placeholder="Own instruction, e.g. “use Du”, “no exclamation marks”"
      onChange={(e) => setText(e.target.value)}
      onBlur={() => text !== value && onSave(text)}
    />
  );
}
