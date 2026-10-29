import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { LogEntry, UpdateInfo } from "../../shared/types";
import type { PageProps } from "../App";
import { Row, Section, Switch } from "../ui";

export function AppPage({ s, save, ov, toast }: PageProps) {
  const [update, setUpdate] = useState<UpdateInfo | null | undefined>(undefined);
  const [busy, setBusy] = useState<"check" | "install" | null>(null);
  const [log, setLog] = useState<LogEntry[]>([]);

  const loadLog = async () => setLog((await invoke<LogEntry[]>("get_log")).reverse());
  useEffect(() => {
    void loadLog();
  }, []);

  const check = async () => {
    setBusy("check");
    try {
      setUpdate(await invoke<UpdateInfo | null>("check_update"));
    } catch (e) {
      toast(`Update check failed: ${e}`);
    }
    setBusy(null);
  };

  const install = async () => {
    setBusy("install");
    try {
      await invoke("install_update");
    } catch (e) {
      toast(`Update failed: ${e}`);
      setBusy(null);
    }
  };

  return (
    <>
      <h1>App</h1>
      <p className="lead">Amber {ov.version}</p>
      <Section>
        <Row label="Start with Windows" hint="Amber waits quietly in the tray until you dictate.">
          <Switch on={s.autostart} onChange={(v) => save((x) => (x.autostart = v))} />
        </Row>
        <Row label="Update automatically" hint="New versions are installed in the background when you are not dictating.">
          <Switch on={s.autoUpdate} onChange={(v) => save((x) => (x.autoUpdate = v))} />
        </Row>
        <Row
          label="Updates"
          hint={
            update === undefined
              ? undefined
              : update
                ? `Version ${update.version} is available.`
                : "Amber is up to date."
          }
        >
          {update ? (
            <button className="btn primary" disabled={busy !== null} onClick={install}>
              {busy === "install" ? "Installing…" : `Install ${update.version}`}
            </button>
          ) : (
            <button className="btn" disabled={busy !== null} onClick={check}>
              {busy === "check" ? "Checking…" : "Check now"}
            </button>
          )}
        </Row>
        <Row label="Project page" hint="Release notes, the installer, and where to report problems.">
          <button className="btn" onClick={() => invoke("open_url", { url: "https://github.com/EinFabo/amberpc/releases" })}>
            Open
          </button>
        </Row>
      </Section>
      <Section title="Diagnostics">
        <Row label="Keep the recognized text in the log" hint="Off: only lengths and timings are logged, never what you said.">
          <Switch on={s.logText} onChange={(v) => save((x) => (x.logText = v))} />
        </Row>
        <div className="row">
          <div className="label">
            <div>Recent events</div>
          </div>
          <button className="btn" onClick={loadLog}>
            Refresh
          </button>
          <button
            className="btn"
            onClick={async () => {
              await navigator.clipboard.writeText(log.map((l) => `${l.time} ${l.level} ${l.message}`).join("\n"));
              toast("Log copied.", true);
            }}
          >
            Copy
          </button>
        </div>
        <div className="log">
          {log.length === 0 && <div className="note">Nothing yet.</div>}
          {log.map((l, i) => (
            <div key={i} className={l.level}>
              <span className="time">{l.time.slice(11, 19)}</span>
              {l.message}
            </div>
          ))}
        </div>
      </Section>
    </>
  );
}
