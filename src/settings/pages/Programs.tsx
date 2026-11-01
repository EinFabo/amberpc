// A profile per program: the writing style the smart cleanup uses there, an own instruction,
// and whether the smart cleanup runs there at all.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { CleanupChoice, Profile, Program, TerminalMode, WritingStyle } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Segmented, Switch } from "../ui";

export const STYLES: [WritingStyle, string][] = [
  ["asSpoken", "As spoken"],
  ["casual", "Casual"],
  ["formal", "Formal"],
  ["concise", "Concise"],
];

const TERMINAL: [TerminalMode, string][] = [
  ["default", "Default"],
  ["off", "Off"],
  ["auto", "Auto"],
  ["bash", "bash"],
  ["powershell", "PowerShell"],
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
      <TerminalSection s={s} save={save} />
      {s.terminal.enabled && <RunCommands s={s} save={save} />}
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
              void save((x) => x.profiles.unshift({ exe: pick, label: label.charAt(0).toUpperCase() + label.slice(1), style: "asSpoken", instruction: "", cleanup: "default", terminal: "default" }));
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
              {s.terminal.enabled && (
                <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
                  <span className="note">Terminal commands</span>
                  <Segmented value={p.terminal ?? "default"} options={TERMINAL} onChange={(v) => update(p.exe, (q) => (q.terminal = v))} />
                </div>
              )}
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

/**
 * Terminal mode: in a terminal, what is said is a request - the cleanup's model turns it into
 * one shell command, typed at the prompt without Enter.
 */
function TerminalSection({ s, save }: Pick<PageProps, "s" | "save">) {
  const [prefixes, setPrefixes] = useState(s.terminal.prefixes.join(", "));
  useEffect(() => setPrefixes(s.terminal.prefixes.join(", ")), [s.terminal.prefixes]);
  return (
    <Section title="Terminal commands">
      <Row
        label="Say what to do, get the command"
        hint="In a terminal (Windows Terminal, PowerShell, Git Bash, PuTTY, WSL) “such alle Dateien, die mit CL anfangen” becomes a bash or PowerShell command, typed at the prompt - never run: you read it and press Enter. A risky one (deleting, sudo...) comes commented out. It uses the cleanup's model; the window's title is its only hint of the folder."
      >
        <Switch on={s.terminal.enabled} onChange={(v) => save((x) => (x.terminal.enabled = v))} />
      </Row>
      {s.terminal.enabled && (
        <Row label="Automatically in terminals" hint="Off: a command only with the command hotkey or a start word - dictating into a terminal is ordinary text. A program's choice below then only sets the shell.">
          <Switch on={s.terminal.automatic} onChange={(v) => save((x) => (x.terminal.automatic = v))} />
        </Row>
      )}
      {s.terminal.enabled && (
        <Row label="Start words" hint="Starting a dictation with one of these makes it a command anywhere: “Befehl: zeig alle Dateien”. Comma-separated. The Hotkeys page has a command hotkey too.">
          <input
            type="text"
            value={prefixes}
            onChange={(e) => setPrefixes(e.target.value)}
            onBlur={() =>
              save((x) => (x.terminal.prefixes = prefixes.split(",").map((p) => p.trim()).filter((p) => p.length > 0)))
            }
          />
        </Row>
      )}
    </Section>
  );
}

/** Behind a warning: Amber presses Enter itself after a harmless command. */
function RunCommands({ s, save }: Pick<PageProps, "s" | "save">) {
  const [asking, setAsking] = useState(false);
  const on = s.terminal.runCommands;
  return (
    <div className="danger-zone">
      <h2>⚠ Danger zone</h2>
      <Row
        label="Run commands by themselves"
        hint="Amber presses Enter after typing the command - it runs before you have read it. A language model can misunderstand you or pick the wrong folder. Risky commands (deleting, overwriting, sudo...) are never run: they still come commented out."
      >
        <Switch on={on} onChange={(v) => (v ? setAsking(true) : save((x) => (x.terminal.runCommands = false)))} />
      </Row>
      {asking && !on && (
        <div className="confirm">
          <span>Commands will run without you checking them first. Switch this on?</span>
          <button className="btn danger" onClick={() => void save((x) => (x.terminal.runCommands = true)).then(() => setAsking(false))}>
            Run them by themselves
          </button>
          <button className="btn" onClick={() => setAsking(false)}>
            Cancel
          </button>
        </div>
      )}
    </div>
  );
}
