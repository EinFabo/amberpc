// The settings window: a short list of pages, each showing its current value. Every change
// is saved at once - there is no "Apply".

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DictationEvent, Overview, Phase, Settings } from "../shared/types";
import { EnginePage } from "./pages/Engine";
import { MicrophonePage } from "./pages/Microphone";
import { HotkeysPage } from "./pages/Hotkeys";
import { BubblePage } from "./pages/Bubble";
import { InsertPage } from "./pages/Insert";
import { AppPage } from "./pages/AppPage";
import { WelcomePage } from "./pages/Welcome";
import { PhonePage } from "./pages/Phone";
import { CleanupPage } from "./pages/Cleanup";
import { ProgramsPage } from "./pages/Programs";
import { HistoryPage } from "./pages/History";
import { StatisticsPage } from "./pages/Statistics";

export interface PageProps {
  ov: Overview;
  s: Settings;
  save: (change: (s: Settings) => void) => Promise<void>;
  refresh: () => Promise<void>;
  go: (page: PageId) => void;
  toast: (message: string, ok?: boolean) => void;
  phase: Phase;
}

export type PageId = "welcome" | "engine" | "cleanup" | "programs" | "microphone" | "hotkeys" | "bubble" | "insert" | "phone" | "history" | "statistics" | "app";

const PAGES: { id: PageId; label: string; value: (o: Overview) => string }[] = [
  { id: "engine", label: "Engine", value: (o) => (o.settings.engine === "local" ? "Local" : providerName(o)) },
  {
    id: "cleanup",
    label: "Cleanup",
    value: (o) => (o.settings.cleanup.smart ? (o.settings.cleanup.engine === "local" ? "Local" : "Server") : o.settings.cleanup.fillers ? "Fillers" : "Off"),
  },
  { id: "programs", label: "Programs", value: (o) => String(o.settings.profiles.length) },
  { id: "microphone", label: "Microphone", value: (o) => o.settings.microphone || "Default" },
  { id: "hotkeys", label: "Hotkeys", value: (o) => o.settings.hotkey.dictate || "None" },
  { id: "bubble", label: "Bubble", value: (o) => ({ always: "Shown", whileDictating: "When used", hidden: "Hidden" })[o.settings.bubble.visibility] },
  { id: "insert", label: "Inserting", value: (o) => (o.settings.insert.method === "paste" ? "Paste" : "Type") },
  { id: "phone", label: "Phone", value: (o) => (o.settings.phone.enabled ? "On" : "Off") },
  { id: "history", label: "History", value: (o) => (o.settings.history ? `${o.settings.historyDays} days` : "Off") },
  { id: "statistics", label: "Statistics", value: (o) => (o.settings.statistics ? "" : "Off") },
  { id: "app", label: "App", value: (o) => `v${o.version}` },
];

function providerName(o: Overview) {
  return o.providers.find((p) => p.id === o.settings.server.provider)?.name ?? "Server";
}

function initialPage(): PageId {
  const p = new URLSearchParams(location.search).get("page");
  return (["welcome", "engine", "cleanup", "programs", "microphone", "hotkeys", "bubble", "insert", "phone", "history", "statistics", "app"] as PageId[]).includes(p as PageId)
    ? (p as PageId)
    : "engine";
}

export function App() {
  const [ov, setOv] = useState<Overview | null>(null);
  const [page, setPage] = useState<PageId>(initialPage);
  const [toastMsg, setToast] = useState<{ text: string; ok: boolean } | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  // The latest settings, so two quick changes in a row build on each other.
  const latest = useRef<Settings | null>(null);
  latest.current = ov?.settings ?? null;

  const refresh = useCallback(async () => {
    const o = await invoke<Overview>("get_overview");
    setOv(o);
    setPhase(o.phase);
  }, []);

  const toast = useCallback((text: string, ok = false) => {
    setToast({ text, ok });
    window.setTimeout(() => setToast((t) => (t?.text === text ? null : t)), ok ? 2500 : 6000);
  }, []);

  useEffect(() => {
    void refresh();
    const subs = [
      listen<Settings>("settings-changed", (e) => setOv((o) => (o ? { ...o, settings: e.payload } : o))),
      listen<string>("navigate", (e) => setPage(e.payload as PageId)),
      listen<DictationEvent>("dictation-state", (e) => setPhase(e.payload.state)),
    ];
    return () => subs.forEach((p) => p.then((un) => un()));
  }, [refresh]);

  const save = useCallback(
    async (change: (s: Settings) => void) => {
      if (!latest.current) return;
      const next: Settings = structuredClone(latest.current);
      change(next);
      latest.current = next;
      setOv((o) => (o ? { ...o, settings: next } : o));
      try {
        const saved = await invoke<Settings>("save_settings", { settings: next });
        setOv((o) => (o ? { ...o, settings: saved } : o));
      } catch (e) {
        toast(String(e));
        void refresh();
      }
    },
    [refresh, toast],
  );

  if (!ov) return null;
  const props: PageProps = { ov, s: ov.settings, save, refresh, go: setPage, toast, phase };

  return (
    <>
      <nav>
        <div className="brand">
          <div className="disc" />
          Amber
        </div>
        {PAGES.map((p) => (
          <button key={p.id} className={page === p.id ? "active" : ""} onClick={() => setPage(p.id)}>
            {p.label}
            <span className="value">{p.value(ov)}</span>
          </button>
        ))}
        <div className="spacer" />
        <button className={page === "welcome" ? "active" : ""} onClick={() => setPage("welcome")}>
          Setup guide
        </button>
        <div className="status">{phaseLabel(phase)}</div>
      </nav>
      <main>
        {page === "welcome" && <WelcomePage {...props} />}
        {page === "engine" && <EnginePage {...props} />}
        {page === "cleanup" && <CleanupPage {...props} />}
        {page === "programs" && <ProgramsPage {...props} />}
        {page === "microphone" && <MicrophonePage {...props} />}
        {page === "hotkeys" && <HotkeysPage {...props} />}
        {page === "bubble" && <BubblePage {...props} />}
        {page === "insert" && <InsertPage {...props} />}
        {page === "phone" && <PhonePage {...props} />}
        {page === "history" && <HistoryPage {...props} />}
        {page === "statistics" && <StatisticsPage {...props} />}
        {page === "app" && <AppPage {...props} />}
      </main>
      {toastMsg && <div className={`toast ${toastMsg.ok ? "ok" : ""}`}>{toastMsg.text}</div>}
    </>
  );
}

function phaseLabel(p: Phase) {
  return {
    idle: "Ready",
    recording: "Listening…",
    transcribing: "Recognizing…",
    inserting: "Inserting…",
    done: "Done",
    error: "Failed",
  }[p];
}
